use super::Sample;
use anyhow::{Context, Result, bail, ensure};
use redb::TableDefinition;
use switchbot_meter::{AlertState, Event, Policy};

pub(super) const FORMAT: u8 = 1;
pub(super) const IDENTITY: TableDefinition<(), (u8, &str)> = TableDefinition::new("meter");
pub(super) type ReadingFields = (i64, i16, u8, u8);
pub(super) const SAMPLES: TableDefinition<i64, ReadingFields> =
    TableDefinition::new("minute_samples");
pub(super) const LATEST: TableDefinition<(), ReadingFields> = TableDefinition::new("latest");
pub(super) type StateFields = (i64, Option<i64>, bool, bool, i64);
pub(super) const STATE: TableDefinition<(), StateFields> = TableDefinition::new("monitor_state");
pub(super) const OUTBOX: TableDefinition<i64, (u8, i64)> =
    TableDefinition::new("pending_notifications");

impl Sample {
    pub(super) fn fields(&self) -> ReadingFields {
        (self.received, self.tenths_c, self.humidity, self.battery)
    }

    pub(super) fn from_fields((received, tenths_c, humidity, battery): ReadingFields) -> Self {
        Self {
            received,
            tenths_c,
            humidity,
            battery,
        }
    }

    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            self.received >= 0
                && (-1279..=1279).contains(&self.tenths_c)
                && self.humidity <= 100
                && self.battery <= 100,
            "invalid stored meter reading"
        );
        Ok(())
    }
}

pub(super) struct Checkpoint {
    pub state: AlertState,
    pub sequence: i64,
}

impl Checkpoint {
    pub fn load(
        raw: Option<StateFields>,
        latest: Option<ReadingFields>,
        policy: Policy,
    ) -> Result<Self> {
        let (started, pending_since, high, lost, sequence) =
            raw.context("missing receiver state")?;
        Ok(Self {
            state: AlertState {
                policy,
                started,
                received: latest.map(|r| r.0),
                pending_since,
                high,
                lost,
            },
            sequence,
        })
    }

    pub fn fields(&self) -> StateFields {
        (
            self.state.started,
            self.state.pending_since,
            self.state.high,
            self.state.lost,
            self.sequence,
        )
    }
}

pub(super) fn event_number(event: Event) -> u8 {
    match event {
        Event::High => 0,
        Event::Normal => 1,
        Event::Lost => 2,
        Event::Received => 3,
    }
}

pub(super) fn event_from_number(number: u8) -> Result<Event> {
    Ok(match number {
        0 => Event::High,
        1 => Event::Normal,
        2 => Event::Lost,
        3 => Event::Received,
        _ => bail!("unknown stored notification kind {number}"),
    })
}
