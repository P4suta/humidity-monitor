use super::{Sample, schema::*};
use anyhow::{Context, Result, bail, ensure};
use redb::WriteTransaction;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::path::Path;
use switchbot_meter::{Address, Event};

fn flag(raw: i64) -> Result<bool> {
    match raw {
        0 => Ok(false),
        1 => Ok(true),
        _ => bail!("invalid SQLite state flag"),
    }
}

pub(super) fn import(path: &Path, target: &WriteTransaction, address: Address) -> Result<()> {
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let source = db.transaction()?;
    let integrity: String = source.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(
        integrity == "ok",
        "SQLite history failed its integrity check"
    );
    let mut monitor = source.prepare("SELECT id,address,started,received,temperature,humidity,battery,pending_since,high,lost FROM monitor")?;
    let mut rows = monitor.query([])?;
    let row = rows.next()?.context("missing SQLite meter")?;
    let id: i64 = row.get(0)?;
    let stored: String = row.get(1)?;
    ensure!(
        id == 1 && stored.parse::<Address>().ok() == Some(address),
        "SQLite database belongs to a different meter"
    );
    let started: i64 = row.get(2)?;
    let received: Option<i64> = row.get(3)?;
    let pending: Option<i64> = row.get(7)?;
    let high = flag(row.get(8)?)?;
    let lost = flag(row.get(9)?)?;
    ensure!(
        started >= 0
            && received.is_none_or(|at| at >= started)
            && pending.is_none_or(|at| received.is_some_and(|last| (started..=last).contains(&at))),
        "invalid SQLite receiver times"
    );
    match (received, row.get(4)?, row.get(5)?, row.get(6)?) {
        (Some(at), Some(temperature), Some(humidity), Some(battery)) => {
            let sample = Sample::from_fields((at, temperature, humidity, battery));
            sample.validate()?;
            target.open_table(LATEST)?.insert((), sample.fields())?;
        }
        (None, None, None, None) => (),
        _ => bail!("incomplete SQLite reception"),
    }
    ensure!(rows.next()?.is_none(), "multiple SQLite meters");
    let mut query = source.prepare(
        "SELECT minute,received,temperature,humidity,battery FROM samples ORDER BY minute",
    )?;
    let mut rows = query.query([])?;
    let mut samples = target.open_table(SAMPLES)?;
    while let Some(row) = rows.next()? {
        let minute: i64 = row.get(0)?;
        let sample = Sample::from_fields((row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?));
        sample.validate()?;
        ensure!(
            sample.received / 60 == minute
                && received.is_some_and(|last| (started..=last).contains(&sample.received)),
            "invalid SQLite sample time"
        );
        samples.insert(minute, sample.fields())?;
    }
    let sequence: i64 = source
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name='outbox'",
            [],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    ensure!(sequence >= 0, "invalid SQLite notification sequence");
    let mut query = source.prepare("SELECT id,kind,at FROM outbox ORDER BY id")?;
    let mut rows = query.query([])?;
    let mut outbox = target.open_table(OUTBOX)?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let kind: String = row.get(1)?;
        let at: i64 = row.get(2)?;
        ensure!(
            (1..=sequence).contains(&id) && at >= started,
            "invalid SQLite notification"
        );
        let event = match kind.as_str() {
            "high" => Event::High,
            "normal" => Event::Normal,
            "lost" => Event::Lost,
            "received" => Event::Received,
            _ => bail!("unknown SQLite notification kind"),
        };
        outbox.insert(id, (event_number(event), at))?;
    }
    target
        .open_table(STATE)?
        .insert((), (started, pending, high, lost, sequence))?;
    Ok(())
}
