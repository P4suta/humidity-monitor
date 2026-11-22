use humidity_monitor::bluez::Advertisements;
use std::collections::HashMap;
use switchbot_meter::Address;
use zbus::zvariant::{OwnedValue, Value};

fn property<K>(key: K, bytes: &[u8]) -> OwnedValue
where
    K: std::hash::Hash + Eq + zbus::zvariant::Type + Into<Value<'static>>,
{
    let bytes = OwnedValue::try_from(Value::from(bytes.to_vec())).unwrap();
    OwnedValue::from(HashMap::from([(key, bytes)]))
}

const PATH: &str = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF";
const UUID: &str = "0000fd3d-0000-1000-8000-00805f9b34fb";
const DATA: &[u8] = &[0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x36, 3, 2, 0x96, 55, 0];

#[test]
fn only_fresh_advertisement_parts_on_the_selected_adapter_advance_reception() {
    let target: Address = "AA:BB:CC:DD:EE:FF".parse().unwrap();
    let mut ads = Advertisements::new("hci0", Some(target));
    let svc = || {
        HashMap::from([(
            "ServiceData".into(),
            property(UUID.to_owned(), &[b'w', 0, 100]),
        )])
    };
    let mfr = || HashMap::from([("ManufacturerData".into(), property(2409_u16, DATA))]);
    assert!(ads.update(PATH, svc(), &[], 0).is_none());
    assert!(ads.update(PATH, mfr(), &[], 3000).is_none());
    assert!(
        ads.update(PATH, HashMap::new(), &["ManufacturerData".into()], 3001)
            .is_none()
    );
    assert!(ads.update(PATH, svc(), &[], 3002).is_none());
    assert!(
        ads.update(&PATH.replace("hci0", "hci1"), mfr(), &[], 3003)
            .is_none()
    );
    assert!(
        ads.update(&PATH.replace("_FF", "_01"), mfr(), &[], 3003)
            .is_none()
    );
    let (_, value) = ads.update(PATH, mfr(), &[], 3004).unwrap();
    assert_eq!(
        (value.tenths_c(), value.humidity(), value.battery()),
        (222, 55, 100)
    );
    assert!(ads.update(PATH, mfr(), &[], 3005).is_none());
    assert!(
        ads.update(
            PATH,
            HashMap::from([("RSSI".into(), OwnedValue::from(-45i16))]),
            &[],
            3006
        )
        .is_none()
    );
}
