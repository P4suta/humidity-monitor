#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub high_above: u8,
    pub high_seconds: i64,
    pub recovery_seconds: i64,
    pub loss_seconds: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    High,
    Normal,
    Lost,
    Received,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertState {
    pub policy: Policy,
    pub started: i64,
    pub received: Option<i64>,
    pub pending_since: Option<i64>,
    pub high: bool,
    pub lost: bool,
}

impl AlertState {
    pub fn new(started: i64, policy: Policy) -> Self {
        Self {
            policy,
            started,
            received: None,
            pending_since: None,
            high: false,
            lost: false,
        }
    }

    pub fn advance(&mut self, now: i64, humidity: Option<u8>) -> Vec<Event> {
        let mut events = Vec::new();
        let previous = self.received.unwrap_or(self.started);
        if now < previous {
            return events;
        }
        if now / 60 - previous / 60 > 1 {
            self.pending_since = None;
        }
        if !self.lost && now - previous >= self.policy.loss_seconds {
            self.lost = true;
            events.push(Event::Lost);
        }
        if let Some(value) = humidity {
            self.received = Some(now);
            if self.lost {
                self.lost = false;
                events.push(Event::Received);
            }
            if (value > self.policy.high_above) == self.high {
                self.pending_since = None;
            } else {
                let since = *self.pending_since.get_or_insert(now);
                let duration = if self.high {
                    self.policy.recovery_seconds
                } else {
                    self.policy.high_seconds
                };
                if now - since >= duration {
                    self.high = !self.high;
                    self.pending_since = None;
                    events.push(if self.high {
                        Event::High
                    } else {
                        Event::Normal
                    });
                }
            }
        }
        events
    }
}
