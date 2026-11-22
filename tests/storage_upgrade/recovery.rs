use super::*;
use std::{
    fs,
    os::unix::fs::symlink,
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[test]
fn interrupted_import_restarts_from_the_intact_sqlite_snapshot() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let old = dir.path().join("meter.sqlite");
    let target = dir.path().join("meter.redb");
    let stage = target.with_added_extension("humidity-stage");
    let source = legacy(&old)?;
    source.execute_batch("PRAGMA journal_mode=DELETE; BEGIN EXCLUSIVE")?;
    let config = dir.path().join("monitor.toml");
    fs::write(
        &config,
        format!(
            "address='{ADDRESS}'\ndatabase='{}'\n[policy]\nhigh_above=55\nhigh_seconds=1800\nrecovery_seconds=300\nloss_seconds=600\n",
            target.display()
        ),
    )?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_humidity-monitor"))
        .arg("serve")
        .arg(config)
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !stage.join("partial").exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    let interrupted = stage.join("partial").is_file() && !target.exists();
    let _ = child.kill();
    child.wait()?;
    assert!(interrupted);
    source.execute_batch("ROLLBACK")?;
    assert!(!target.exists());
    let db = open(&target)?;
    assert_eq!(db.latest()?.unwrap().received, 125);
    assert_eq!(db.history(0, 130, 3)?.len(), 2);
    assert_eq!(db.pending()?, Some((5, Event::High, 60)));
    assert!(!stage.exists());
    assert_eq!(
        source.query_row("SELECT COUNT(*) FROM samples", [], |r| r.get::<_, i64>(0))?,
        2
    );
    Ok(())
}

#[test]
fn a_broken_link_to_legacy_history_cannot_create_an_empty_receiver() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let old = dir.path().join("meter.sqlite");
    let target = dir.path().join("meter.redb");
    symlink(dir.path().join("missing"), &old)?;
    assert!(open(&target).is_err());
    assert!(!target.exists());
    assert!(old.is_symlink());
    Ok(())
}

#[test]
fn restoring_a_closed_database_preserves_history_state_and_notification_ids() -> Result<()> {
    let source = tempfile::tempdir()?;
    let backup = tempfile::tempdir()?;
    let old = source.path().join("history.sqlite");
    let live = source.path().join("history.redb");
    let restored = backup.path().join("restore.redb");
    drop(legacy(&old)?);
    let mut store = open(&live)?;
    for id in 5..=8 {
        store.acknowledge(id)?;
    }
    store.tick(725)?;
    let rows = store.history(0, 130, 8)?;
    let latest = store.latest()?;
    drop(store);
    fs::copy(live, &restored)?;
    drop(source);
    let mut db = open(&restored)?;
    assert_eq!(db.history(0, 130, 8)?, rows);
    assert_eq!(db.latest()?, latest);
    assert!(db.state()?.lost);
    assert_eq!(db.pending()?, Some((43, Event::Lost, 725)));
    db.acknowledge(8)?;
    assert_eq!(db.pending()?, Some((43, Event::Lost, 725)));
    Ok(())
}

#[test]
fn invalid_legacy_fields_leave_the_source_repairable() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let old = dir.path().join("meter.sqlite");
    let target = dir.path().join("meter.redb");
    let db = legacy(&old)?;
    for edit in [
        "UPDATE samples SET minute=10 WHERE minute=1",
        "UPDATE samples SET battery=101 WHERE minute=1",
        "UPDATE samples SET temperature=-1280 WHERE minute=1",
        "UPDATE samples SET received=-1 WHERE minute=1",
        "UPDATE monitor SET humidity=NULL",
        "UPDATE monitor SET high=2",
        "UPDATE monitor SET pending_since=126",
        "UPDATE monitor SET received=100",
        "UPDATE monitor SET address='AA:BB:CC:DD:EE:01'",
        "UPDATE outbox SET kind='other'",
        "UPDATE outbox SET at=-1",
        "UPDATE sqlite_sequence SET seq=4 WHERE name='outbox'",
    ] {
        let backup = tempfile::tempdir()?;
        let copy = backup.path().join("meter.sqlite");
        db.execute("VACUUM INTO ?1", [copy.to_str().unwrap()])?;
        Connection::open(&copy)?.execute_batch(edit)?;
        assert!(open(&copy).is_err(), "accepted {edit}");
        assert!(!copy.with_extension("redb").exists());
        assert!(copy.is_file());
    }
    assert_eq!(open(&target)?.history(0, 130, 2)?.len(), 2);
    Ok(())
}

#[test]
fn exhausting_notification_ids_rolls_back_the_entire_reception() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let old = dir.path().join("meter.sqlite");
    let source = legacy(&old)?;
    source.execute(
        "UPDATE sqlite_sequence SET seq=?1 WHERE name='outbox'",
        [i64::MAX - 1],
    )?;
    let target = old.with_extension("redb");
    let mut db = open(&target)?;
    let reading = switchbot_meter::Reading::decode(
        ADDRESS.parse().unwrap(),
        ADDRESS.parse().unwrap(),
        &[b'w', 0, 99],
        &[0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0, 0, 2, 0x96, 55, 0],
    )
    .unwrap();
    assert!(db.record(800, reading).is_err());
    drop(db);
    let mut db = open(&target)?;
    assert_eq!(db.latest()?.unwrap().received, 125);
    assert_eq!(db.history(0, 801, 8)?.len(), 2);
    assert!(!db.state()?.lost);
    for id in 5..=8 {
        db.acknowledge(id)?;
    }
    assert_eq!(db.pending()?, None);
    Ok(())
}
