use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use humidity_monitor::{Config, Publisher, Store};
use std::sync::{Arc, Mutex};
use switchbot_meter::{Event, Policy};

type Requests = Arc<Mutex<Vec<(String, String)>>>;

const EXAMPLE: &str = r#"
address = "AA:BB:CC:DD:EE:FF"
database = "readings.sqlite"
[policy]
high_above = 80
high_seconds = 300
recovery_seconds = 60
loss_seconds = 90
"#;

#[test]
fn installations_can_set_all_display_and_monitoring_options() -> anyhow::Result<()> {
    let c = Config::parse(EXAMPLE)?;
    assert_eq!(c.policy.high_above, 80);
    assert_eq!(c.retention_days, 365);
    assert!(c.reference.is_none() && c.dry_below.is_none() && c.ntfy.is_none());
    assert!(c.listen.ip().is_loopback());
    let overridden = format!(
        "label='Laboratory'\ntimezone='Europe/Paris'\nreference=[42,60]\ndry_below=22\nretention_days=30\n{EXAMPLE}"
    );
    let c = Config::parse(&overridden)?;
    assert_eq!(
        (
            c.label.as_str(),
            c.timezone.iana_name().unwrap(),
            c.reference,
            c.dry_below,
            c.retention_days
        ),
        ("Laboratory", "Europe/Paris", Some([42, 60]), Some(22), 30)
    );
    for invalid in [
        EXAMPLE.replace("high_above = 80", "high_above = 101"),
        EXAMPLE.replace("loss_seconds = 90", "loss_seconds = 0"),
        format!("reference=[50,40]\n{EXAMPLE}"),
        format!("dry_below=101\n{EXAMPLE}"),
        format!("adapter='../hci0'\n{EXAMPLE}"),
    ] {
        assert!(Config::parse(&invalid).is_err());
    }
    Ok(())
}

#[tokio::test]
async fn failed_delivery_keeps_the_transition_and_retry_uses_bearer_auth() -> anyhow::Result<()> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let router = Router::new()
        .route(
            "/test-topic",
            post(
                |State(calls): State<Requests>, headers: HeaderMap, body: String| async move {
                    let mut calls = calls.lock().unwrap();
                    calls.push((headers["authorization"].to_str().unwrap().to_owned(), body));
                    if calls.len() == 1 {
                        (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
                    } else {
                        (StatusCode::OK, r#"{"id":"test","event":"message"}"#)
                    }
                },
            ),
        )
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/test-topic", listener.local_addr()?);
    tokio::spawn(async { axum::serve(listener, router).await.unwrap() });
    let temp = tempfile::tempdir()?;
    let token = temp.path().join("token");
    std::fs::write(&token, "test-publisher\n")?;
    let mut store = Store::open(
        &temp.path().join("meter.sqlite"),
        "AA:BB:CC:DD:EE:FF".parse().unwrap(),
        0,
        Policy {
            high_above: 60,
            high_seconds: 30,
            recovery_seconds: 10,
            loss_seconds: 90,
        },
        365,
    )?;
    store.tick(90)?;
    let (_, event, at) = store.pending()?.unwrap();
    assert_eq!(event, Event::Lost);
    let publisher = Publisher::new(&url, &token, "Laboratory")?;
    assert!(publisher.send(event, at).await.is_err());
    assert!(store.pending()?.is_some());
    publisher.send(event, at).await?;
    store.acknowledge(store.pending()?.unwrap().0)?;
    assert!(store.pending()?.is_none());
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0], calls[1]);
    assert_eq!(calls[1].0, "Bearer test-publisher");
    assert!(calls[1].1.contains("Laboratory") && calls[1].1.contains("受信途絶"));
    Ok(())
}
