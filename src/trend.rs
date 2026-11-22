use crate::Sample;

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Bin {
    pub first: i64,
    pub last: i64,
    pub low: u8,
    pub high: u8,
    pub first_rh: u8,
    pub last_rh: u8,
}

pub(crate) fn group<E>(
    samples: impl Iterator<Item = Result<Sample, E>>,
    start: i64,
    end: i64,
) -> Result<Vec<Bin>, E> {
    let width = (end.saturating_sub(start) / 60 / 300).max(1);
    let bucket = |minute: i64| (minute - start / 60) / width;
    let mut bins: Vec<Bin> = Vec::new();
    for sample in samples {
        let sample = sample?;
        let minute = sample.received / 60;
        let rh = sample.humidity;
        if let Some(last) = bins.last_mut()
            && last.last / 60 + 1 == minute
            && bucket(last.last / 60) == bucket(minute)
        {
            last.last = sample.received;
            last.low = last.low.min(rh);
            last.high = last.high.max(rh);
            last.last_rh = rh;
        } else {
            bins.push(Bin {
                first: sample.received,
                last: sample.received,
                low: rh,
                high: rh,
                first_rh: rh,
                last_rh: rh,
            });
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
        assert_eq!((bins[0].first, bins[0].last), (start, start + 6 * 60));
        assert_eq!(
            (bins[1].first, bins[1].last),
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
            .map(|b| (b.first, b.last, b.first_rh, b.last_rh, b.low, b.high))
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
