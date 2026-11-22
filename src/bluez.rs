use anyhow::{Result, bail, ensure};
use futures_util::TryStreamExt;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use switchbot_meter::{Address, Reading};
use zbus::{
    Connection, MatchRule, MessageStream, Proxy,
    fdo::DBusProxy,
    message::Type,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

const DEVICE: &str = "org.bluez.Device1";
const SERVICE: &str = "0000fd3d-0000-1000-8000-00805f9b34fb";
type Properties = HashMap<String, OwnedValue>;

#[derive(Default)]
struct Parts {
    service: Option<Vec<u8>>,
    manufacturer: Option<Vec<u8>>,
    since: u64,
}

pub struct Advertisements {
    prefix: String,
    target: Option<Address>,
    pending: HashMap<Address, Parts>,
}

impl Advertisements {
    pub fn new(adapter: &str, target: Option<Address>) -> Self {
        Self {
            prefix: format!("/org/bluez/{adapter}/dev_"),
            target,
            pending: HashMap::new(),
        }
    }

    pub fn update(
        &mut self,
        path: &str,
        mut props: Properties,
        invalidated: &[String],
        ms: u64,
    ) -> Option<(Address, Reading)> {
        let address: Address = path
            .strip_prefix(&self.prefix)?
            .replace('_', ":")
            .parse()
            .ok()?;
        if self.target.is_some_and(|target| target != address) {
            return None;
        }
        if invalidated
            .iter()
            .any(|key| matches!(key.as_str(), "ManufacturerData" | "ServiceData"))
        {
            self.pending.remove(&address);
        }
        let service = props.remove("ServiceData");
        let manufacturer = props.remove("ManufacturerData");
        if service.is_none() && manufacturer.is_none() {
            return None;
        }
        self.pending
            .retain(|_, p| ms.saturating_sub(p.since) <= 2000);
        let parts = self.pending.entry(address).or_insert_with(|| Parts {
            since: ms,
            ..Parts::default()
        });
        if let Some(data) = service {
            parts.service = HashMap::<String, OwnedValue>::try_from(data)
                .ok()
                .and_then(|mut map| map.remove(SERVICE))
                .and_then(|bytes| Vec::try_from(bytes).ok());
        }
        if let Some(data) = manufacturer {
            parts.manufacturer = HashMap::<u16, OwnedValue>::try_from(data)
                .ok()
                .and_then(|mut map| map.remove(&2409))
                .and_then(|bytes| Vec::try_from(bytes).ok());
        }
        let result = Reading::decode(
            self.target.unwrap_or(address),
            address,
            parts.service.as_ref()?,
            parts.manufacturer.as_ref()?,
        );
        self.pending.remove(&address);
        result.map(|reading| (address, reading))
    }
}

pub async fn scan(
    connection: &Connection,
    adapter: &str,
    target: Option<Address>,
    mut received: impl FnMut(Address, Reading) -> Result<()>,
) -> Result<()> {
    ensure!(crate::valid_adapter(adapter), "invalid adapter name");
    let owner = DBusProxy::new(connection)
        .await?
        .get_name_owner("org.bluez".try_into()?)
        .await?;
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(owner.as_str())?
        .build();
    let mut signals = MessageStream::for_match_rule(rule, connection, Some(256)).await?;
    let path = format!("/org/bluez/{adapter}");
    let proxy = Proxy::new(
        connection,
        owner.as_str(),
        path.as_str(),
        "org.bluez.Adapter1",
    )
    .await?;
    let filter = HashMap::from([
        ("Transport", Value::from("le")),
        ("DuplicateData", Value::from(true)),
    ]);
    proxy
        .call::<_, _, ()>("SetDiscoveryFilter", &(filter,))
        .await?;
    proxy.call::<_, _, ()>("StartDiscovery", &()).await?;
    let mut ads = Advertisements::new(adapter, target);
    let started = Instant::now();
    let mut health = tokio::time::interval(Duration::from_secs(15));
    loop {
        let message = tokio::select! {
            result=signals.try_next()=>result?.ok_or_else(||anyhow::anyhow!("BlueZ signal stream closed"))?,
            _=health.tick()=>{
                let properties=Proxy::new(connection,owner.as_str(),path.as_str(),"org.freedesktop.DBus.Properties").await?;
                let discovering:OwnedValue=properties.call("Get",&("org.bluez.Adapter1","Discovering")).await?;
                ensure!(bool::try_from(discovering)?,"BlueZ discovery stopped");
                continue;
            }
        };
        let header = message.header();
        let body = message.body();
        let observation = match (
            header.interface().map(|n| n.as_str()),
            header.member().map(|n| n.as_str()),
        ) {
            (Some("org.freedesktop.DBus.Properties"), Some("PropertiesChanged")) => {
                let (interface, properties, invalidated): (String, Properties, Vec<String>) =
                    body.deserialize()?;
                if interface != DEVICE {
                    continue;
                }
                let Some(device_path) = header.path() else {
                    continue;
                };
                (device_path.to_string(), properties, invalidated)
            }
            (Some("org.freedesktop.DBus.ObjectManager"), Some("InterfacesAdded")) => {
                let (device_path, mut interfaces): (OwnedObjectPath, HashMap<String, Properties>) =
                    body.deserialize()?;
                let Some(properties) = interfaces.remove(DEVICE) else {
                    continue;
                };
                (device_path.to_string(), properties, Vec::new())
            }
            (Some("org.freedesktop.DBus.ObjectManager"), Some("InterfacesRemoved")) => {
                let (object, interfaces): (OwnedObjectPath, Vec<String>) = body.deserialize()?;
                if object.as_str() == path && interfaces.iter().any(|n| n == "org.bluez.Adapter1") {
                    bail!("Bluetooth adapter removed")
                }
                if interfaces.iter().any(|n| n == DEVICE)
                    && let Some(addr) = object
                        .as_str()
                        .strip_prefix(&ads.prefix)
                        .and_then(|p| p.replace('_', ":").parse::<Address>().ok())
                {
                    ads.pending.remove(&addr);
                }
                continue;
            }
            _ => continue,
        };
        if let Some((address, reading)) = ads.update(
            &observation.0,
            observation.1,
            &observation.2,
            started.elapsed().as_millis() as u64,
        ) {
            received(address, reading)?;
        }
    }
}
