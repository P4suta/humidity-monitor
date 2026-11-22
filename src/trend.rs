use crate::Sample;

const COLUMNS: i64 = 300;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Point {
    pub at: i64,
    pub rh: u8,
}

impl From<Sample> for Point {
    fn from(sample: Sample) -> Self {
        Self {
            at: sample.received,
            rh: sample.humidity,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Extrema {
    pub low: Point,
    pub high: Point,
}

impl Extrema {
    fn include(&mut self, other: Self) {
        if other.low.rh < self.low.rh {
            self.low = other.low;
        }
        if other.high.rh > self.high.rh {
            self.high = other.high;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Bin {
    pub first: Point,
    pub last: Point,
    pub range: Extrema,
}

impl Bin {
    fn single(point: Point) -> Self {
        Self {
            first: point,
            last: point,
            range: Extrema {
                low: point,
                high: point,
            },
        }
    }
}

pub(crate) fn group<E>(
    samples: impl Iterator<Item = Result<Sample, E>>,
    start: i64,
    end: i64,
) -> Result<Vec<Bin>, E> {
    let width = (end.saturating_sub(start) / 60 / COLUMNS).max(1);
    let bucket = |minute: i64| (minute - start / 60) / width;
    let mut bins: Vec<Bin> = Vec::new();
    for sample in samples {
        let point = Point::from(sample?);
        let minute = point.at / 60;
        let next = Bin::single(point);
        if let Some(last) = bins.last_mut()
            && last.last.at / 60 + 1 == minute
            && bucket(last.last.at / 60) == bucket(minute)
        {
            last.last = point;
            last.range.include(next.range);
        } else {
            bins.push(next);
        }
    }
    Ok(bins)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_chart_buckets_at_the_requested_utc_minute() {
        let start = 1_700_000_103;
        let rows = (0..10).map(|minute| {
            Ok::<_, ()>(Sample {
                received: start + minute * 60,
                humidity: 50,
                tenths_c: 200,
                battery: 99,
            })
        });
        let bins = group(rows, start, start + 300 * 7 * 60).unwrap();
        assert_eq!(bins.len(), 2);
        assert_eq!((bins[0].first.at, bins[0].last.at), (start, start + 6 * 60));
        assert_eq!(
            (bins[1].first.at, bins[1].last.at),
            (start + 7 * 60, start + 9 * 60)
        );
    }

    #[test]
    fn splits_gaps_and_bucket_boundaries_without_losing_extrema_or_endpoints() {
        let rows = [
            (60, 45),
            (125, 32),
            (180, 80),
            (245, 50),
            (365, 42),
            (425, 41),
            (600, 39),
            (660, 46),
        ];
        let rows = rows.map(|(received, humidity)| {
            Ok::<_, ()>(Sample {
                received,
                humidity,
                tenths_c: 0,
                battery: 100,
            })
        });
        let result = group(rows.into_iter(), 60, 60 + 300 * 6 * 60).unwrap();
        let actual: Vec<_> = result
            .iter()
            .map(|b| {
                (
                    b.first.at,
                    b.last.at,
                    b.first.rh,
                    b.last.rh,
                    b.range.low.rh,
                    b.range.high.rh,
                )
            })
            .collect();
        assert_eq!(
            actual,
            [
                (60, 245, 45, 50, 32, 80),
                (365, 365, 42, 42, 42, 42),
                (425, 425, 41, 41, 41, 41),
                (600, 660, 39, 46, 39, 46)
            ]
        );
    }
}
