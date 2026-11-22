use crate::Sample;

const COLUMNS: i64 = 48;

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

enum Interior {
    Empty,
    Run(Bin),
    Extrema(Extrema),
}

struct Column {
    first: Bin,
    interior: Interior,
    last: Option<Bin>,
}

impl Column {
    fn new(first: Point) -> Self {
        Self {
            first: Bin::single(first),
            interior: Interior::Empty,
            last: None,
        }
    }

    fn push(&mut self, point: Point) {
        let run = self.last.as_mut().unwrap_or(&mut self.first);
        let next = Bin::single(point);
        if run.last.at / 60 + 1 == point.at / 60 {
            run.last = point;
            run.range.include(next.range);
        } else if let Some(finished) = self.last.replace(next) {
            match &mut self.interior {
                Interior::Empty => self.interior = Interior::Run(finished),
                Interior::Run(run) => {
                    let mut range = run.range;
                    range.include(finished.range);
                    self.interior = Interior::Extrema(range);
                }
                Interior::Extrema(range) => range.include(finished.range),
            }
        }
    }

    fn finish(self, bins: &mut Vec<Bin>) {
        bins.push(self.first);
        match self.interior {
            Interior::Empty => (),
            Interior::Run(run) => bins.push(run),
            Interior::Extrema(range) => {
                let mut points = [range.low, range.high];
                points.sort_unstable_by_key(|p| p.at);
                bins.push(Bin::single(points[0]));
                if points[0].at != points[1].at {
                    bins.push(Bin::single(points[1]));
                }
            }
        }
        bins.extend(self.last);
    }
}

pub(crate) fn group<E>(
    samples: impl Iterator<Item = Result<Sample, E>>,
    start: i64,
    end: i64,
) -> Result<Vec<Bin>, E> {
    let first_minute = start.div_euclid(60);
    let span = end.div_euclid(60) - first_minute;
    let width = ((span + COLUMNS - 1) / COLUMNS).max(1);
    let column = |at: i64| ((at.div_euclid(60) - first_minute) / width).min(COLUMNS - 1);
    let mut samples = samples.peekable();
    let mut bins = Vec::new();
    while let Some(first) = samples.next().transpose()? {
        let index = column(first.received);
        let mut current = Column::new(first.into());
        while let Some(row) =
            samples.next_if(|row| row.as_ref().is_ok_and(|s| column(s.received) == index))
        {
            current.push(row?.into());
        }
        current.finish(&mut bins);
    }
    Ok(bins)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(minute: i64, humidity: u8) -> Sample {
        Sample {
            received: minute * 60 + 7,
            humidity,
            tenths_c: 200,
            battery: 99,
        }
    }

    fn bounds(bins: &[Bin]) -> Vec<(i64, i64, u8, u8)> {
        bins.iter()
            .map(|b| {
                (
                    b.first.at / 60,
                    b.last.at / 60,
                    b.range.low.rh,
                    b.range.high.rh,
                )
            })
            .collect()
    }

    #[test]
    fn marks_observed_interior_extrema_and_keeps_the_outer_runs() {
        let points = [
            (0, 41),
            (1, 42),
            (3, 50),
            (4, 97),
            (5, 51),
            (8, 20),
            (9, 3),
            (10, 25),
            (12, 40),
            (13, 43),
        ];
        let rows = points.into_iter().map(|(m, h)| Ok::<_, ()>(sample(m, h)));
        let bins = group(rows, 0, COLUMNS * 20 * 60).unwrap();
        assert_eq!(
            bounds(&bins),
            [
                (0, 1, 41, 42),
                (4, 4, 97, 97),
                (9, 9, 3, 3),
                (12, 13, 40, 43)
            ]
        );
    }

    #[test]
    fn bounds_a_fragmented_year_and_includes_the_last_minute() {
        let last = 365 * 1440;
        let rows = (0..=last)
            .step_by(2)
            .map(|m| Ok::<_, ()>(sample(m, (m % 101) as u8)));
        let bins = group(rows, 7, last * 60 + 7).unwrap();
        assert!(bins.len() <= 48 * 4, "{} chart entries", bins.len());
        assert_eq!(bins.first().unwrap().first.at, 7);
        assert_eq!(bins.last().unwrap().last.at, last * 60 + 7);
        assert!(bins.iter().all(|b| b.first == b.last));
    }

    #[test]
    fn a_constant_interior_range_has_one_marker_at_an_observed_time() {
        let rows = [0, 3, 5, 7, 9].map(|m| Ok::<_, ()>(sample(m, 50)));
        let bins = group(rows.into_iter(), 0, COLUMNS * 10 * 60).unwrap();
        assert_eq!(
            bounds(&bins),
            [(0, 0, 50, 50), (3, 3, 50, 50), (9, 9, 50, 50)]
        );
    }

    #[test]
    fn every_twelve_minute_reception_pattern_preserves_gaps_and_extrema() {
        for mask in 0..1_u16 << 12 {
            let rows: Vec<_> = (0..12)
                .filter(|&m| mask & (1 << m) != 0)
                .map(|m| sample(m, ((m * 37) % 101) as u8))
                .collect();
            let result = group(
                rows.iter().copied().map(Ok::<_, ()>),
                7,
                COLUMNS * 12 * 60 + 7,
            )
            .unwrap();
            let runs: Vec<_> = rows
                .chunk_by(|a, b| b.received - a.received == 60)
                .collect();
            assert!(result.len() <= 4, "mask {mask}");
            assert_eq!(
                result.first().map(|b| b.first.at),
                rows.first().map(|r| r.received)
            );
            assert_eq!(
                result.last().map(|b| b.last.at),
                rows.last().map(|r| r.received)
            );
            assert_eq!(
                result.iter().map(|b| b.range.low.rh).min(),
                rows.iter().map(|s| s.humidity).min()
            );
            assert_eq!(
                result.iter().map(|b| b.range.high.rh).max(),
                rows.iter().map(|s| s.humidity).max()
            );
            assert!(result.windows(2).all(|p| p[0].last.at < p[1].first.at));
            for b in &result {
                for point in [b.first, b.last, b.range.low, b.range.high] {
                    assert!(
                        rows.iter()
                            .any(|r| r.received == point.at && r.humidity == point.rh)
                    );
                }
                assert!(runs.iter().any(
                    |r| r[0].received <= b.first.at && b.last.at <= r.last().unwrap().received
                ));
            }
            if runs.len() <= 3 {
                assert_eq!(result.len(), runs.len());
                for (b, r) in result.iter().zip(runs) {
                    assert_eq!(b.first.at, r[0].received);
                    assert_eq!(b.last.at, r.last().unwrap().received);
                }
            }
        }
    }

    #[test]
    fn includes_both_edges_within_forty_eight_intervals() {
        for minutes in [47, 48, 49, 95, 96, 97, 1440] {
            let rows = (0..=minutes).map(|m| Ok::<_, ()>(sample(m, 50)));
            let bins = group(rows, 7, minutes * 60 + 7).unwrap();
            assert!(bins.len() <= 48);
            assert_eq!(bins[0].first.at, 7);
            assert_eq!(bins.last().unwrap().last.at, minutes * 60 + 7);
        }
    }

    #[test]
    fn a_storage_error_anywhere_in_a_column_fails_the_chart() {
        for position in 0..5 {
            let rows = (0..5).map(|m| {
                if m == position {
                    Err("read failed")
                } else {
                    Ok(sample(m, 50))
                }
            });
            assert_eq!(group(rows, 7, COLUMNS * 10 * 60 + 7), Err("read failed"));
        }
    }

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
        let bins = group(rows, start, start + COLUMNS * 7 * 60).unwrap();
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
        let result = group(rows.into_iter(), 60, 60 + COLUMNS * 6 * 60).unwrap();
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
