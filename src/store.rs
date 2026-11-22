use std::path::Path;

use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, Row, params};
use switchbot_meter::{Address, AlertState, Event, Policy, Reading};

#[derive(Debug, Eq, PartialEq)]
pub struct Sample {
    pub received: i64,
    pub tenths_c: i16,
    pub humidity: u8,
    pub battery: u8,
}

impl Sample {
    fn row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            received: row.get(0)?,
            tenths_c: row.get(1)?,
            humidity: row.get(2)?,
            battery: row.get(3)?,
        })
    }
}

pub struct Store {
    pub(crate) db: Connection,
    policy: Policy,
    retention: i64,
    notifications: bool,
}

impl Store {
    pub fn open(
        path: &Path,
        address: Address,
        now: i64,
        policy: Policy,
        retention_days: u32,
    ) -> Result<Self> {
        ensure!(
            retention_days > 0 && now >= 0,
            "invalid retention or start time"
        );
        let mut db = Connection::open(path)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        let tx = db.transaction()?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS monitor (
            id INTEGER PRIMARY KEY CHECK(id=1), address TEXT NOT NULL, started INTEGER NOT NULL,
            received INTEGER, temperature INTEGER, humidity INTEGER, battery INTEGER,
            pending_since INTEGER, high INTEGER NOT NULL DEFAULT 0, lost INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS samples (minute INTEGER PRIMARY KEY, received INTEGER NOT NULL,
                temperature INTEGER NOT NULL, humidity INTEGER NOT NULL, battery INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS outbox (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, at INTEGER NOT NULL);")?;
        tx.execute(
            "INSERT OR IGNORE INTO monitor(id,address,started) VALUES(1,?1,?2)",
            params![address.to_string(), now],
        )?;
        let stored: String = tx.query_row("SELECT address FROM monitor", [], |r| r.get(0))?;
        ensure!(
            stored == address.to_string(),
            "database belongs to a different meter"
        );
        tx.commit()?;
        Ok(Self {
            db,
            policy,
            retention: i64::from(retention_days) * 86400,
            notifications: true,
        })
    }

    pub fn with_notifications(mut self, enabled: bool) -> Self {
        self.notifications = enabled;
        self
    }

    pub fn record(&mut self, now: i64, reading: Reading) -> Result<()> {
        self.advance(now, Some(reading))
    }

    pub fn tick(&mut self, now: i64) -> Result<()> {
        self.advance(now, None)
    }

    fn advance(&mut self, now: i64, reading: Option<Reading>) -> Result<()> {
        let tx = self.db.transaction()?;
        let state = Self::load(&tx, self.policy)?;
        if now < state.received.unwrap_or(state.started) {
            return Ok(());
        }
        let mut next = state.clone();
        for event in next.advance(now, reading.map(Reading::humidity)) {
            if !self.notifications {
                continue;
            }
            let kind = match event {
                Event::High => "high",
                Event::Normal => "normal",
                Event::Lost => "lost",
                Event::Received => "received",
            };
            tx.execute(
                "INSERT INTO outbox(kind,at) VALUES(?1,?2)",
                params![kind, now],
            )?;
        }
        if let Some(value) = reading {
            let fields = params![now, value.tenths_c(), value.humidity(), value.battery()];
            tx.execute("INSERT OR IGNORE INTO samples(minute,received,temperature,humidity,battery) VALUES(?1/60,?1,?2,?3,?4)",fields)?;
            tx.execute(
                "UPDATE monitor SET received=?1,temperature=?2,humidity=?3,battery=?4 WHERE id=1",
                fields,
            )?;
        }
        if next.pending_since != state.pending_since
            || next.high != state.high
            || next.lost != state.lost
        {
            tx.execute(
                "UPDATE monitor SET pending_since=?1,high=?2,lost=?3 WHERE id=1",
                params![next.pending_since, next.high, next.lost],
            )?;
        }
        let cutoff = now.saturating_sub(self.retention);
        tx.execute(
            "DELETE FROM samples WHERE minute <= ?1/60 AND received < ?1",
            [cutoff],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn load(db: &Connection, policy: Policy) -> rusqlite::Result<AlertState> {
        db.query_row(
            "SELECT started,received,pending_since,high,lost FROM monitor WHERE id=1",
            [],
            |r| {
                Ok(AlertState {
                    policy,
                    started: r.get(0)?,
                    received: r.get(1)?,
                    pending_since: r.get(2)?,
                    high: r.get(3)?,
                    lost: r.get(4)?,
                })
            },
        )
    }

    pub fn state(&self) -> Result<AlertState> {
        Ok(Self::load(&self.db, self.policy)?)
    }

    pub fn latest(&self) -> Result<Option<Sample>> {
        Ok(self.db.query_row("SELECT received,temperature,humidity,battery FROM monitor WHERE received IS NOT NULL",[],Sample::row).optional()?)
    }

    pub fn history(&self, start: i64, end: i64, limit: u32) -> Result<Vec<Sample>> {
        let mut stmt = self.db.prepare("SELECT received,temperature,humidity,battery FROM samples
            WHERE minute BETWEEN ?1/60 AND ?2/60 AND received>=?1 AND received<?2 ORDER BY minute DESC LIMIT ?3")?;
        Ok(stmt
            .query_map(params![start, end, limit], Sample::row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn pending(&self) -> Result<Option<(i64, Event, i64)>> {
        Ok(self
            .db
            .query_row(
                "SELECT id,kind,at FROM outbox ORDER BY id LIMIT 1",
                [],
                |r| {
                    let kind: String = r.get(1)?;
                    let event = match kind.as_str() {
                        "high" => Event::High,
                        "normal" => Event::Normal,
                        "lost" => Event::Lost,
                        "received" => Event::Received,
                        _ => return Err(rusqlite::Error::InvalidQuery),
                    };
                    Ok((r.get(0)?, event, r.get(2)?))
                },
            )
            .optional()?)
    }

    pub fn acknowledge(&mut self, id: i64) -> Result<()> {
        self.db.execute("DELETE FROM outbox WHERE id=?1", [id])?;
        Ok(())
    }
}
