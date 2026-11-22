use std::{path::Path, sync::Arc};

use anyhow::{Context, Result, ensure};
use redb::{Database, ReadTransaction, ReadableDatabase, ReadableTable, WriteTransaction};
use switchbot_meter::{Address, AlertState, Event, Policy, Reading};

mod files;
mod legacy;
mod schema;
use schema::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sample {
    pub received: i64,
    pub tenths_c: i16,
    pub humidity: u8,
    pub battery: u8,
}

#[derive(Clone)]
pub struct Store {
    db: Arc<Database>,
    policy: Policy,
    retention: i64,
    notifications: bool,
}

pub(crate) fn require_database_file(path: &Path) -> Result<()> {
    let name = path.as_os_str().as_encoded_bytes();
    ensure!(
        !name.is_empty() && name != b":memory:" && !name.starts_with(b"file:"),
        "database must name a persistent filesystem path"
    );
    Ok(())
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
        require_database_file(path)?;
        let db = files::open(path, address, now)?;
        Ok(Self {
            db: Arc::new(db),
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

    fn advance(&self, now: i64, reading: Option<Reading>) -> Result<()> {
        let tx = self.db.begin_write()?;
        if self.stage(&tx, now, reading)? {
            tx.commit()?;
        }
        Ok(())
    }

    fn stage(&self, tx: &WriteTransaction, now: i64, reading: Option<Reading>) -> Result<bool> {
        let mut states = tx.open_table(STATE)?;
        let mut latest = tx.open_table(LATEST)?;
        let mut checkpoint = Checkpoint::load(
            states.get(())?.map(|v| v.value()),
            latest.get(())?.map(|v| v.value()),
            self.policy,
        )?;
        if now
            < checkpoint
                .state
                .received
                .unwrap_or(checkpoint.state.started)
        {
            return Ok(false);
        }
        let previous = checkpoint.fields();
        let events = checkpoint
            .state
            .advance(now, reading.map(Reading::humidity));
        if self.notifications && !events.is_empty() {
            let mut outbox = tx.open_table(OUTBOX)?;
            for event in events {
                checkpoint.sequence = checkpoint
                    .sequence
                    .checked_add(1)
                    .context("notification sequence exhausted")?;
                outbox.insert(checkpoint.sequence, (event_number(event), now))?;
            }
        }
        let state_changed = previous != checkpoint.fields();
        if state_changed {
            states.insert((), checkpoint.fields())?;
        }
        let mut samples = tx.open_table(SAMPLES)?;
        if let Some(value) = reading {
            let sample = (now, value.tenths_c(), value.humidity(), value.battery());
            if let redb::Entry::Vacant(entry) = samples.entry(now / 60)? {
                entry.insert(sample)?;
            }
            latest.insert((), sample)?;
        }
        let cutoff = now.saturating_sub(self.retention);
        let expired = samples.first()?.is_some_and(|(_, v)| v.value().0 < cutoff);
        if expired {
            samples.retain_in(..=cutoff / 60, |_, sample| sample.0 >= cutoff)?;
        }
        Ok(state_changed || reading.is_some() || expired)
    }

    pub(crate) fn snapshot(&self) -> Result<Snapshot> {
        Ok(Snapshot {
            tx: self.db.begin_read()?,
            policy: self.policy,
        })
    }

    pub fn state(&self) -> Result<AlertState> {
        self.snapshot()?.state()
    }
    pub fn latest(&self) -> Result<Option<Sample>> {
        self.snapshot()?.latest()
    }
    pub fn history(&self, start: i64, end: i64, limit: u32) -> Result<Vec<Sample>> {
        self.snapshot()?.history(start, end, limit)
    }

    pub fn pending(&self) -> Result<Option<(i64, Event, i64)>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(OUTBOX)?;
        table
            .first()?
            .map(|(key, value)| {
                let (number, at) = value.value();
                Ok((key.value(), event_from_number(number)?, at))
            })
            .transpose()
    }

    pub fn acknowledge(&mut self, id: i64) -> Result<()> {
        let tx = self.db.begin_write()?;
        let removed = tx.open_table(OUTBOX)?.remove(id)?.is_some();
        if removed {
            tx.commit()?;
        }
        Ok(())
    }
}

pub(crate) struct Snapshot {
    tx: ReadTransaction,
    policy: Policy,
}

impl Snapshot {
    pub(crate) fn state(&self) -> Result<AlertState> {
        let raw = self.tx.open_table(STATE)?.get(())?.map(|v| v.value());
        let latest = self.tx.open_table(LATEST)?.get(())?.map(|v| v.value());
        Ok(Checkpoint::load(raw, latest, self.policy)?.state)
    }

    pub(crate) fn latest(&self) -> Result<Option<Sample>> {
        Ok(self
            .tx
            .open_table(LATEST)?
            .get(())?
            .map(|v| Sample::from_fields(v.value())))
    }

    pub(crate) fn history(&self, start: i64, end: i64, limit: u32) -> Result<Vec<Sample>> {
        if start >= end || limit == 0 {
            return Ok(Vec::new());
        }
        let samples = self.tx.open_table(SAMPLES)?;
        let mut rows = Vec::new();
        for entry in samples.range(start / 60..=end / 60)?.rev() {
            let sample = Sample::from_fields(entry?.1.value());
            if (start..end).contains(&sample.received) {
                rows.push(sample);
                if rows.len() == limit as usize {
                    break;
                }
            }
        }
        Ok(rows)
    }

    pub(crate) fn bins(&self, start: i64, end: i64) -> Result<Vec<crate::trend::Bin>> {
        if start > end {
            return Ok(Vec::new());
        }
        let samples = self.tx.open_table(SAMPLES)?;
        let rows = samples
            .range(start / 60..=end / 60)?
            .filter_map(|entry| match entry {
                Ok((_, value)) => {
                    let sample = Sample::from_fields(value.value());
                    (start..=end)
                        .contains(&sample.received)
                        .then_some(Ok(sample))
                }
                Err(error) => Some(Err(error)),
            });
        Ok(crate::trend::group(rows, start, end)?)
    }
}
