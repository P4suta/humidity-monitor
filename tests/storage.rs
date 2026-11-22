use humidity_monitor::Store;
use switchbot_meter::{Address, Event, Policy, Reading};

const MAC: &str = "AA:BB:CC:DD:EE:FF";
const POLICY: Policy = Policy {
    high_above: 55,
    high_seconds: 1800,
    recovery_seconds: 300,
    loss_seconds: 600,
};
const DATA: [u8; 12] = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x36, 3, 2, 0x96, 55, 0];

fn reading(humidity: u8) -> Reading {
    let address: Address = MAC.parse().unwrap();
    let mut data = DATA;
    data[10] = humidity;
    Reading::decode(address, address, &[b'w', 0, 100], &data).unwrap()
}

#[test]
fn sqlite_restart_keeps_minute_history_live_reception_and_pending_notifications()
-> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("meter.sqlite");
    let address = MAC.parse().unwrap();
    let mut store = Store::open(&path, address, 0, POLICY, 365)?;
    store.record(1, reading(55))?;
    store.record(59, reading(56))?;
    assert_eq!(store.latest()?.unwrap().received, 59);
    assert_eq!(store.history(0, 60, 10)?[0].humidity, 55);
    for at in (60..=1800).step_by(60) {
        store.record(at, reading(56))?;
    }
    drop(store);
    let mut store = Store::open(&path, address, 1859, POLICY, 365)?;
    store.record(1859, reading(56))?;
    let (id, event, at) = store.pending()?.unwrap();
    assert_eq!((event, at), (Event::High, 1859));
    assert_eq!(store.history(0, 1860, 100)?.len(), 31);
    drop(store);
    let mut store = Store::open(&path, address, 3000, POLICY, 365)?;
    store.tick(3000)?;
    assert_eq!(store.pending()?.unwrap().0, id);
    store.acknowledge(id)?;
    let (lost, event, _) = store.pending()?.unwrap();
    assert_eq!(event, Event::Lost);
    store.record(3001, reading(40))?;
    store.acknowledge(lost)?;
    assert_eq!(store.pending()?.unwrap().1, Event::Received);
    assert!(
        Store::open(
            &path,
            "AA:BB:CC:DD:EE:01".parse().unwrap(),
            3002,
            POLICY,
            365
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn expiry_removes_samples_after_365_days_even_during_an_outage() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(
        &temp.path().join("meter.sqlite"),
        MAC.parse().unwrap(),
        1,
        POLICY,
        365,
    )?;
    store.record(1, reading(45))?;
    store.record(61, reading(46))?;
    store.record(60, reading(34))?;
    assert_eq!(store.latest()?.unwrap().humidity, 46);
    store.tick(365 * 86400 + 2)?;
    assert_eq!(store.history(0, i64::MAX, 100)?.len(), 1);
    assert_eq!(store.latest()?.unwrap().received, 61);
    Ok(())
}
