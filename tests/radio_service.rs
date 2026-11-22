use anyhow::{Context, Result, ensure};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use switchbot_meter::{Address, Reading};
use zbus::{
    connection::Builder,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

struct Bus(Child);
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Adapter(Arc<AtomicBool>);
#[zbus::interface(name = "org.bluez.Adapter1")]
impl Adapter {
    fn set_discovery_filter(&self, filter: HashMap<String, OwnedValue>) -> zbus::fdo::Result<()> {
        if filter
            .get("Transport")
            .and_then(|v| <&str>::try_from(v).ok())
            != Some("le")
            || filter
                .get("DuplicateData")
                .and_then(|v| bool::try_from(v).ok())
                != Some(true)
        {
            return Err(zbus::fdo::Error::InvalidArgs(
                "expected duplicate LE data".into(),
            ));
        }
        Ok(())
    }
    fn start_discovery(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    #[zbus(property)]
    fn discovering(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn isolated_bluez_signals_reach_sqlite_and_incomplete_updates_do_not_refresh_it() -> Result<()>
{
    let mut daemon = Bus(Command::new("dbus-daemon")
        .args([
            "--session",
            "--address=unix:tmpdir=/tmp",
            "--nofork",
            "--print-address=1",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .context("start private D-Bus daemon")?);
    let mut bus_address = String::new();
    BufReader::new(daemon.0.stdout.take().unwrap()).read_line(&mut bus_address)?;
    let address = bus_address.trim();
    let started = Arc::new(AtomicBool::new(false));
    let bluez = Builder::address(address)?
        .name("org.bluez")?
        .serve_at("/org/bluez/hci0", Adapter(started.clone()))?
        .build()
        .await?;
    let client = Builder::address(address)?.build().await?;
    let target: Address = "AA:BB:CC:DD:EE:FF".parse().unwrap();
    let (seen, mut events) = tokio::sync::mpsc::unbounded_channel::<Reading>();
    let scan = tokio::spawn(async move {
        humidity_monitor::bluez::scan(&client, "hci0", Some(target), |_, r| Ok(seen.send(r)?)).await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !started.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    let path = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF";
    let service = HashMap::from([(
        "0000fd3d-0000-1000-8000-00805f9b34fb",
        Value::from(vec![b'w', 0, 100]),
    )]);
    let manufacturer = HashMap::from([(
        2409u16,
        Value::from(vec![
            0xaa_u8, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x36, 3, 2, 0x96, 55, 0,
        ]),
    )]);
    let props = HashMap::from([
        ("ManufacturerData", Value::from(manufacturer)),
        ("ServiceData", Value::from(service)),
    ]);
    let object = OwnedObjectPath::try_from(path)?;
    bluez
        .emit_signal(
            None::<&str>,
            "/",
            "org.freedesktop.DBus.ObjectManager",
            "InterfacesAdded",
            &(
                object,
                HashMap::from([("org.bluez.Device1", props.clone())]),
            ),
        )
        .await?;
    let first = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await?
        .context("radio task stopped")?;
    let cfg = humidity_monitor::Config::parse(
        "address='AA:BB:CC:DD:EE:FF'\ndatabase='meter.sqlite'\n[policy]\nhigh_above=55\nhigh_seconds=1800\nrecovery_seconds=300\nloss_seconds=600",
    )?;
    let dir = tempfile::tempdir()?;
    let mut store = humidity_monitor::Store::open(
        &dir.path().join("meter.sqlite"),
        target,
        0,
        cfg.policy,
        365,
    )?;
    store.record(100, first)?;
    let emit = |path, changed| {
        bluez.emit_signal(
            None::<&str>,
            path,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            changed,
        )
    };
    let incomplete = (
        "org.bluez.Device1",
        HashMap::from([("ServiceData", props["ServiceData"].clone())]),
        Vec::<String>::new(),
    );
    emit(path, &incomplete).await?;
    let complete = ("org.bluez.Device1", props, Vec::<String>::new());
    emit("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_01", &complete).await?;
    ensure!(
        tokio::time::timeout(Duration::from_millis(200), events.recv())
            .await
            .is_err(),
        "incomplete update refreshed reception"
    );
    assert_eq!(store.latest()?.unwrap().received, 100);
    emit(path, &complete).await?;
    let second = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await?
        .context("collector stopped")?;
    store.record(180, second)?;
    assert_eq!(store.history(0, 181, 10)?.len(), 2);
    scan.abort();
    Ok(())
}
