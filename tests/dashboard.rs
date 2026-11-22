use anyhow::{Result, ensure};
use humidity_monitor::{Config, Store, dashboard};
use reqwest::Client;
use serde_json::{Value, json};
use std::{
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use switchbot_meter::Reading;
use tokio::net::TcpListener;

struct Driver(Child);
impl Drop for Driver {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn element(client: &Client, session: &str, selector: &str) -> Result<String> {
    let response: Value = client
        .post(format!("{session}/element"))
        .json(&json!({"using":"css selector","value":selector}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(response["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .unwrap()
        .to_owned())
}
async fn text(client: &Client, session: &str, selector: &str) -> Result<String> {
    let id = element(client, session, selector).await?;
    let response: Value = client
        .get(format!("{session}/element/{id}/text"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(response["value"].as_str().unwrap().to_owned())
}

#[tokio::test(flavor = "multi_thread")]
async fn mobile_browser_navigates_all_ranges_and_exposes_missing_history() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = Config::parse(
        "address='AA:BB:CC:DD:EE:FF'\ndatabase='meter.sqlite'\nlabel='試験用温湿度計'\nreference=[40,50]\ndry_below=35\n[policy]\nhigh_above=55\nhigh_seconds=1800\nrecovery_seconds=300\nloss_seconds=600",
    )?;
    let now = humidity_monitor::unix_now()?;
    let mut store = Store::open(
        &dir.path().join("meter.sqlite"),
        cfg.address,
        now - 90000,
        cfg.policy,
        365,
    )?;
    for (seconds, rh) in [
        (87000, 51),
        (2400, 45),
        (2340, 49),
        (2280, 46),
        (1200, 33),
        (1140, 34),
    ] {
        let b = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x36, 3, 2, 0x96, rh, 0];
        store.record(
            now - seconds,
            Reading::decode(cfg.address, cfg.address, &[b'w', 0, 100], &b).unwrap(),
        )?;
    }
    let app = dashboard(Arc::new(Mutex::new(store)), Arc::new(cfg));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
    let port = std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port();
    let exe = std::env::var_os("CHROMEDRIVER").unwrap_or_else(|| "chromedriver".into());
    let _driver = Driver(
        Command::new(exe)
            .arg(format!("--port={port}"))
            .stdout(Stdio::null())
            .spawn()?,
    );
    let client = Client::builder().timeout(Duration::from_secs(20)).build()?;
    let driver = format!("http://127.0.0.1:{port}");
    for _ in 0..100 {
        if client.get(format!("{driver}/status")).send().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let response:Value=client.post(format!("{driver}/session")).json(&json!({"capabilities":{"alwaysMatch":{"browserName":"chrome","goog:chromeOptions":{"args":["--headless","--no-sandbox","--disable-dev-shm-usage"],"mobileEmulation":{"deviceMetrics":{"width":320,"height":700,"pixelRatio":2}}}}}})).send().await?.json().await?;
    let id = response["value"]["sessionId"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("WebDriver session: {response}"))?;
    let session = format!("{driver}/session/{id}");
    client
        .post(format!("{session}/url"))
        .json(&json!({"url":url}))
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(text(&client, &session, "h1").await?, "試験用温湿度計");
    let status = text(&client, &session, "#current").await?;
    assert!(
        status.contains("34")
            && status.contains("受信途絶")
            && status.contains("乾燥")
            && status.contains("最終受信")
    );
    let stamp = element(&client, &session, "#current time[datetime]").await?;
    let stamp: Value = client
        .get(format!("{session}/element/{stamp}/attribute/datetime"))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(
        stamp["value"]
            .as_str()
            .unwrap()
            .parse::<jiff::Timestamp>()?
            .as_second(),
        now - 1140
    );
    assert!(text(&client, &session, "table").await?.is_empty());
    for (range, label) in [
        ("24h", "24時間"),
        ("7d", "7日"),
        ("30d", "30日"),
        ("1y", "1年"),
    ] {
        let link = element(&client, &session, &format!("nav a[data-period='{range}']")).await?;
        client
            .post(format!("{session}/element/{link}/click"))
            .json(&json!({}))
            .send()
            .await?
            .error_for_status()?;
        assert_eq!(
            text(&client, &session, "a[aria-current='page']").await?,
            label
        );
        assert!(text(&client, &session, "table").await?.is_empty());
        let history = element(&client, &session, "a.history-link").await?;
        client
            .post(format!("{session}/element/{history}/click"))
            .json(&json!({}))
            .send()
            .await?
            .error_for_status()?;
        element(&client, &session, "tbody tr time[datetime]").await?;
        let table = text(&client, &session, "table").await?;
        ensure!(
            table.contains("欠測")
                && table.contains("34％")
                && table.contains("51％") == (range != "24h"),
            "table range or gap incorrect: {table}"
        );
        element(&client, &session, "svg[role='img']").await?;
        element(&client, &session, "svg .reference").await?;
        let pieces: Value = client
            .post(format!("{session}/elements"))
            .json(&json!({"using":"css selector","value":"svg path.series"}))
            .send()
            .await?
            .json()
            .await?;
        assert_eq!(pieces["value"].as_array().unwrap().len(), 2, "{range}");
        element(&client, &session, "svg path.series[d*='C']").await?;
        let markers: Value = client
            .post(format!("{session}/elements"))
            .json(&json!({"using":"css selector","value":"svg circle"}))
            .send()
            .await?
            .json()
            .await?;
        assert_eq!(
            markers["value"].as_array().unwrap().len(),
            usize::from(range != "24h")
        );
    }
    client.delete(session).send().await?.error_for_status()?;
    Ok(())
}
