#[path = "storage_upgrade/endurance.rs"]
mod endurance;
#[path = "storage_upgrade/recovery.rs"]
mod recovery;

use anyhow::Result;
use humidity_monitor::Store;
use rusqlite::Connection;
use std::path::Path;
use switchbot_meter::{Address, Event, Policy};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
const POLICY: Policy = Policy {
    high_above: 55,
    high_seconds: 1800,
    recovery_seconds: 300,
    loss_seconds: 600,
};

fn open(path: &Path) -> Result<Store> {
    Store::open(path, ADDRESS.parse::<Address>().unwrap(), 9000, POLICY, 365)
}

fn legacy(path: &Path) -> Result<Connection> {
    let db = Connection::open(path)?;
    db.execute_batch("PRAGMA journal_mode=WAL;
        PRAGMA wal_autocheckpoint=0;
        CREATE TABLE monitor (id INTEGER PRIMARY KEY, address TEXT, started INTEGER, received INTEGER, temperature INTEGER, humidity INTEGER, battery INTEGER, pending_since INTEGER, high INTEGER, lost INTEGER);
        CREATE TABLE samples (minute INTEGER PRIMARY KEY, received INTEGER, temperature INTEGER, humidity INTEGER, battery INTEGER);
        CREATE TABLE outbox (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT, at INTEGER);
        INSERT INTO monitor VALUES (1,'AA:BB:CC:DD:EE:FF',0,125,-7,56,99,120,0,0);
        INSERT INTO samples VALUES (1,65,-10,55,100),(2,121,-8,54,99);
        INSERT INTO outbox VALUES (5,'high',60),(6,'normal',64),(7,'lost',70),(8,'received',90),(42,'normal',100);
        DELETE FROM outbox WHERE id=42;")?;
    Ok(db)
}

#[test]
fn upgrades_a_wal_snapshot_and_keeps_the_sequence_after_reopening() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let old = dir.path().join("readings.sqlite");
    let new = dir.path().join("readings.redb");
    let source = legacy(&old)?;
    let mut store = open(&new)?;
    let rows = store.history(0, 130, 5)?;
    assert_eq!(
        rows.iter()
            .map(|r| (r.received, r.humidity))
            .collect::<Vec<_>>(),
        [(121, 54), (65, 55)]
    );
    let last = store.latest()?.unwrap();
    assert_eq!(
        (last.received, last.tenths_c, last.humidity, last.battery),
        (125, -7, 56, 99)
    );
    assert_eq!(store.state()?.pending_since, Some(120));
    for (id, event, at) in [
        (5, Event::High, 60),
        (6, Event::Normal, 64),
        (7, Event::Lost, 70),
        (8, Event::Received, 90),
    ] {
        assert_eq!(store.pending()?, Some((id, event, at)));
        store.acknowledge(id)?;
    }
    drop(store);
    let mut store = open(&old)?;
    assert_eq!(store.pending()?, None);
    store.tick(725)?;
    assert_eq!(store.pending()?, Some((43, Event::Lost, 725)));
    assert_eq!(
        source.query_row("SELECT COUNT(*) FROM outbox", [], |row| row
            .get::<_, i64>(0))?,
        4
    );
    Ok(())
}

#[test]
fn upgrades_an_idle_meter_and_preserves_an_active_loss() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("standby.sqlite");
    let source = legacy(&path)?;
    source.execute_batch("DELETE FROM samples; DELETE FROM outbox; UPDATE monitor SET received=NULL,temperature=NULL,humidity=NULL,battery=NULL,pending_since=NULL,high=0,lost=1;")?;
    let store = open(&path)?;
    assert!(store.latest()?.is_none());
    assert!(store.history(0, i64::MAX, 1)?.is_empty());
    let state = store.state()?;
    assert!(state.lost);
    assert!(!state.high);
    assert_eq!(state.received, None);
    Ok(())
}

#[test]
fn a_custom_legacy_name_still_resolves_after_archiving_the_original() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let old = dir.path().join("indoor.history");
    drop(legacy(&old)?);
    let destination = old.with_added_extension("redb");
    let first = open(&old)?.latest()?;
    assert!(destination.is_file());
    std::fs::rename(&old, dir.path().join("archived.sqlite"))?;
    assert_eq!(open(&old)?.latest()?, first);
    assert_eq!(open(&destination)?.latest()?, first);
    assert!(!old.exists());
    Ok(())
}

#[test]
fn refuses_a_foreign_database_or_a_truncated_copy_of_meter_history() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let other = dir.path().join("other.redb");
    drop(redb::Database::create(&other)?);
    assert!(open(&other).is_err());
    let old = dir.path().join("source.sqlite");
    drop(legacy(&old)?);
    drop(open(&old)?);
    let damaged = dir.path().join("damaged.redb");
    std::fs::copy(old.with_extension("redb"), &damaged)?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(&damaged)?
        .set_len(4096)?;
    assert!(open(&damaged).is_err());
    assert_eq!(std::fs::metadata(&damaged)?.len(), 4096);
    Ok(())
}

#[test]
fn failed_imports_preserve_the_source_and_never_publish_partial_history() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let old = dir.path().join("readings.sqlite");
    let target = dir.path().join("readings.redb");
    let db = legacy(&old)?;
    db.execute("UPDATE samples SET humidity=101 WHERE minute=2", [])?;
    assert!(open(&target).is_err());
    assert!(!target.exists());
    db.execute("UPDATE samples SET humidity=87 WHERE minute=2", [])?;
    assert_eq!(open(&target)?.history(0, 130, 1)?[0].humidity, 87);
    Ok(())
}

#[test]
fn opens_the_published_redb_layout_and_refuses_an_unknown_format() -> Result<()> {
    use redb::TableDefinition;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("archived.redb");
    let meter: TableDefinition<(), (u8, &str)> = TableDefinition::new("meter");
    let current: TableDefinition<(), (i64, i16, u8, u8)> = TableDefinition::new("latest");
    let minutes: TableDefinition<i64, (i64, i16, u8, u8)> = TableDefinition::new("minute_samples");
    let state: TableDefinition<(), (i64, Option<i64>, bool, bool, i64)> =
        TableDefinition::new("monitor_state");
    let notices: TableDefinition<i64, (u8, i64)> = TableDefinition::new("pending_notifications");
    let db = redb::Database::create(&path)?;
    let tx = db.begin_write()?;
    tx.open_table(meter)?.insert((), (2, ADDRESS))?;
    tx.open_table(current)?.insert((), (125, -7, 56, 98))?;
    tx.open_table(minutes)?.insert(2, (121, -8, 55, 99))?;
    tx.open_table(state)?
        .insert((), (0, Some(120), true, false, 48))?;
    for (id, tag) in [(41, 0), (42, 1), (43, 2), (44, 3)] {
        tx.open_table(notices)?.insert(id, (tag, 65))?;
    }
    tx.commit()?;
    drop(db);
    let error = open(&path).err().expect("accepted an unknown format");
    assert!(
        error
            .to_string()
            .contains("unsupported meter history format")
    );
    let db = redb::Database::open(&path)?;
    let tx = db.begin_write()?;
    tx.open_table(meter)?.insert((), (1, ADDRESS))?;
    tx.commit()?;
    drop(db);
    let mut store = open(&path)?;
    let row = store.history(120, 126, 1)?[0];
    assert_eq!(
        (row.received, row.tenths_c, row.humidity, row.battery),
        (121, -8, 55, 99)
    );
    assert_eq!(store.latest()?.unwrap().received, 125);
    assert!(store.state()?.high);
    assert_eq!(store.state()?.pending_since, Some(120));
    for (id, event) in [
        (41, Event::High),
        (42, Event::Normal),
        (43, Event::Lost),
        (44, Event::Received),
    ] {
        assert_eq!(store.pending()?, Some((id, event, 65)));
        store.acknowledge(id)?;
    }
    store.tick(725)?;
    assert_eq!(store.pending()?, Some((49, Event::Lost, 725)));
    Ok(())
}

#[test]
fn damaged_destinations_never_trigger_an_empty_store_or_a_second_import() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let old = dir.path().join("readings.sqlite");
    let target = dir.path().join("readings.redb");
    let _db = legacy(&old)?;
    for bytes in [b"".as_slice(), b"broken meter history"] {
        std::fs::write(&target, bytes)?;
        assert!(open(&target).is_err());
        assert_eq!(std::fs::read(&target)?, bytes);
    }
    Ok(())
}
