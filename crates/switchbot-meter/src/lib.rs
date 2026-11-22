use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Address([u8; 6]);

impl FromStr for Address {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut octets = [0; 6];
        let mut parts = value.split(':');
        for octet in &mut octets {
            let part = parts
                .next()
                .filter(|s| s.len() == 2 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                .ok_or("expected six colon-separated hexadecimal octets")?;
            *octet = u8::from_str_radix(part, 16).map_err(|_| "invalid address octet")?;
        }
        if parts.next().is_some() {
            return Err("unexpected address octet");
        }
        Ok(Self(octets))
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{a:02X}:{b:02X}:{c:02X}:{d:02X}:{e:02X}:{g:02X}")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reading {
    tenths_c: i16,
    humidity: u8,
    battery: u8,
}

impl Reading {
    pub fn decode(
        target: Address,
        advertised: Address,
        service: &[u8],
        manufacturer: &[u8],
    ) -> Option<Self> {
        let &[b'w' | b'W', _, battery, ..] = service else {
            return None;
        };
        let &[a, b, c, d, e, f, _, _, fraction, degrees, humidity, ..] = manufacturer else {
            return None;
        };
        let battery = battery & 0x7f;
        let humidity = humidity & 0x7f;
        let fraction = fraction & 0x0f;
        if advertised != target
            || Address([a, b, c, d, e, f]) != advertised
            || fraction > 9
            || battery > 100
            || humidity > 100
        {
            return None;
        }
        let magnitude = i16::from(degrees & 0x7f) * 10 + i16::from(fraction);
        let tenths_c = if degrees & 0x80 == 0 {
            -magnitude
        } else {
            magnitude
        };
        Some(Self {
            tenths_c,
            humidity,
            battery,
        })
    }

    pub fn tenths_c(self) -> i16 {
        self.tenths_c
    }
    pub fn humidity(self) -> u8 {
        self.humidity
    }
    pub fn battery(self) -> u8 {
        self.battery
    }
}
