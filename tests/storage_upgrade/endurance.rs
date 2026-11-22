use super::*;
use humidity_monitor::{Config, dashboard, unix_now};
use std::sync::{Arc, Mutex};
use switchbot_meter::Reading;

fn reading(rh: u8) -> Reading {
    let id = ADDRESS.parse().unwrap();
    Reading::decode(
        id,
        id,
        &[b'w', 0, 100],
        &[0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0, 0, 2, 0x96, rh, 0],
    )
    .unwrap()
}

async fn with_year_dashboard(
    store: Store,
    check: impl AsyncFnOnce(String) -> Result<()>,
) -> Result<()> {
    let cfg = Config::parse(
        "address='AA:BB:CC:DD:EE:FF'\ndatabase='year.redb'\n[policy]\nhigh_above=55\nhigh_seconds=1800\nrecovery_seconds=300\nloss_seconds=600",
    )?;
    let app = dashboard(Arc::new(Mutex::new(store)), Arc::new(cfg));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/?period=1y&paused=true", listener.local_addr()?);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    let result = check(url).await;
    stop.send(()).unwrap();
    server.await??;
    result
}

#[test]
#[ignore = "run with just storage-endurance"]
fn ten_thousand_years_of_sparse_receptions_survive_a_restart() -> Result<()> {
    const YEARS: i64 = 10_000;
    const DAYS: u32 = 25 * 146_097;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("centuries.redb");
    let start = 951868800;
    let when = |year: i64| start + (365 * year + year / 4 - year / 100 + year / 400) * 86400;
    let create = || Store::open(&path, ADDRESS.parse().unwrap(), start, POLICY, DAYS + 1);
    let mut store = create()?.with_notifications(false);
    for year in 0..=YEARS {
        store.record(when(year), reading((year % 101) as u8))?;
        if year % 1000 == 999 {
            drop(store);
            store = create()?.with_notifications(false);
        }
    }
    drop(store);
    let mut store = create()?;
    let end = when(YEARS);
    assert_eq!(end - start, i64::from(DAYS) * 86400);
    let rows = store.history(start, end + 1, u32::MAX)?;
    assert_eq!(rows.len(), 10_001);
    for (year, sample) in (0..=YEARS).rev().zip(rows) {
        assert_eq!(
            (sample.received, sample.humidity),
            (when(year), (year % 101) as u8)
        );
    }
    store.tick(end + 86400)?;
    assert_eq!(store.history(start, start + 1, 1)?.len(), 1);
    store.tick(end + 86401)?;
    assert!(store.history(start, start + 1, 1)?.is_empty());
    assert_eq!(store.latest()?.unwrap().received, end);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "run with just storage-endurance"]
async fn dense_year_import_matches_every_sample_and_plots_a_real_gap() -> Result<()> {
    const MINUTES: i64 = 365 * 1440;
    const GAP: i64 = MINUTES / 2;
    let dir = tempfile::tempdir()?;
    let source = dir.path().join("year.sqlite");
    let target = dir.path().join("year.redb");
    let db = legacy(&source)?;
    let now = unix_now()?;
    let start = (now / 60 - MINUTES) * 60 + 1;
    db.execute_batch("DELETE FROM samples; DELETE FROM outbox;")?;
    db.execute("WITH RECURSIVE minutes(i) AS (VALUES (0) UNION ALL SELECT i+1 FROM minutes WHERE i+1 < ?1)
        INSERT INTO samples SELECT (?2+i*60)/60, ?2+i*60, i%100-50, i%101, i%99 FROM minutes WHERE i<>?3",rusqlite::params![MINUTES,start,GAP])?;
    db.execute(
        "UPDATE monitor SET started=?1, received=?2, pending_since=NULL",
        rusqlite::params![start, start + (MINUTES - 1) * 60],
    )?;
    let mut store = open(&target)?;
    let rows = store.history(start, now + 1, u32::MAX)?;
    assert_eq!(rows.len(), (MINUTES - 1) as usize);
    for (row, i) in rows.iter().zip((0..MINUTES).rev().filter(|&i| i != GAP)) {
        assert_eq!(
            (row.received, row.tenths_c, row.humidity, row.battery),
            (
                start + i * 60,
                (i % 100 - 50) as i16,
                (i % 101) as u8,
                (i % 99) as u8
            )
        );
    }
    drop(rows);
    with_year_dashboard(store.clone(), async |url| {
        let chart =
            tokio::spawn(async move { reqwest::get(&url).await?.error_for_status()?.text().await });
        store.record(now, reading(49))?;
        let html = chart.await??;
        assert_eq!(html.matches("<path class=\"series\"").count(), 2);
        assert!(html.contains("以前の測定値"));
        Ok(())
    })
    .await?;
    drop(store);
    let mut db = Store::open(&target, ADDRESS.parse().unwrap(), now, POLICY, 1)?;
    db.tick(now + 60)?;
    let remaining = db.history(0, now + 61, u32::MAX)?;
    assert!(remaining.len() > 1400 && remaining.len() <= 1441);
    assert!(remaining.iter().all(|r| r.received >= now + 60 - 86400));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "run with just storage-endurance"]
async fn intermittent_year_bounds_the_http_response_and_keeps_exact_history_pages() -> Result<()> {
    const COUNT: i64 = 364 * 1440 / 2;
    let dir = tempfile::tempdir()?;
    let source = legacy(&dir.path().join("intermittent.sqlite"))?;
    let last = (unix_now()? / 60 - 5) * 60 + 7;
    source.execute_batch("DELETE FROM samples; DELETE FROM outbox;")?;
    source.execute(
        "WITH RECURSIVE ticks(i) AS (VALUES(0) UNION ALL SELECT i+1 FROM ticks WHERE i+1 < ?1)
         INSERT INTO samples SELECT (?2-i*120)/60, ?2-i*120, 200, i%101, 99 FROM ticks",
        rusqlite::params![COUNT, last],
    )?;
    source.execute(
        "UPDATE monitor SET received=?1, humidity=0, pending_since=NULL",
        [last],
    )?;
    let store = open(&dir.path().join("intermittent.redb"))?;
    with_year_dashboard(store, async |url| {
        let mut page_url = format!("{url}&history=true");
        for page in 0..2 {
            let body = reqwest::get(&page_url)
                .await?
                .error_for_status()?
                .text()
                .await?;
            assert!(body.len() < 128 * 1024, "{} response bytes", body.len());
            assert!(!body.contains("<path class=\"series\""));
            let markers = body.matches("<circle ").count();
            assert!(
                (300..=1200).contains(&markers),
                "{markers} extrema and endpoints"
            );
            let (_, table) = body.split_once("<tbody>").unwrap();
            let (rows, _) = table.split_once("</tbody>").unwrap();
            assert_eq!(rows.matches("<tr class=\"gap\">").count(), 12);
            let mut readings = rows.split("<time datetime=\"").skip(1);
            for i in page * 12..(page + 1) * 12 {
                let stamp = jiff::Timestamp::from_second(last - i * 120)?;
                let (row, _) = readings.next().unwrap().split_once("</tr>").unwrap();
                assert!(row.starts_with(&format!("{stamp}\"")));
                assert!(row.contains(&format!("<td>{}％</td>", i % 101)));
            }
            assert!(readings.next().is_none());
            let before = last - ((page + 1) * 12 - 1) * 120;
            assert!(body.contains(&format!("before={before}#history\"")));
            page_url = format!("{url}&before={before}");
        }
        Ok(())
    })
    .await
}
