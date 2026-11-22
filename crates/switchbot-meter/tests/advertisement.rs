use switchbot_meter::{Address, Reading};

const ADDRESS: &str = "AA:BB:CC:DD:EE:FF";
// The published W3400010 packet masks the six address octets.
const MANUFACTURER: [u8; 12] = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x36, 3, 2, 0x96, 55, 0];
const SERVICE: [u8; 3] = [0x77, 0, 100];

#[test]
fn address_requires_six_two_digit_hexadecimal_octets() {
    for malformed in [
        "A:B:C:D:E:F",
        "AA:BB:CC:DD:EE",
        "AA:BB:CC:DD:EE:FF:00",
        "AA:BB:CC:DD:EE:+F",
        "AA:BB:CC:DD:EE:GG",
        "AA:BB:CC:DD:EE:",
    ] {
        assert!(malformed.parse::<Address>().is_err(), "{malformed}");
    }
    assert_eq!(
        "00:01:09:0a:10:ff".parse::<Address>().unwrap().to_string(),
        "00:01:09:0A:10:FF"
    );
}

#[test]
fn public_advertisement_decodes_for_the_registered_meter_only() {
    let target: Address = ADDRESS.parse().unwrap();
    assert_eq!(target.to_string(), ADDRESS);
    let advertised = "aa:bb:cc:dd:ee:ff".parse().unwrap();
    let reading = Reading::decode(target, advertised, &SERVICE, &MANUFACTURER).unwrap();
    assert_eq!(
        (reading.tenths_c(), reading.humidity(), reading.battery()),
        (222, 55, 100)
    );
    let other = "AA:BB:CC:DD:EE:01".parse().unwrap();
    assert!(Reading::decode(target, other, &SERVICE, &MANUFACTURER).is_none());
}

#[test]
fn incomplete_wrong_model_or_invalid_fields_are_rejected() {
    let target = ADDRESS.parse().unwrap();
    for service in [
        &[][..],
        &SERVICE[..2],
        &[b'i', 0, 100],
        &[0xf7, 0, 100],
        &[b'w', 0, 101],
    ] {
        assert!(Reading::decode(target, target, service, &MANUFACTURER).is_none());
    }
    for (index, value) in [(0, 0x11), (8, 10), (10, 101)] {
        let mut invalid = MANUFACTURER;
        invalid[index] = value;
        assert!(Reading::decode(target, target, &SERVICE, &invalid).is_none());
    }
    assert!(Reading::decode(target, target, &SERVICE, &MANUFACTURER[..10]).is_none());
}

#[test]
fn signed_temperature_and_display_flags_preserve_physical_units() {
    let target = ADDRESS.parse().unwrap();
    let mut data = MANUFACTURER;
    data[8..11].copy_from_slice(&[0xa9, 2, 0x80 | 100]);
    let reading = Reading::decode(target, target, &[b'W', 0, 0x80], &data).unwrap();
    assert_eq!(
        (reading.tenths_c(), reading.humidity(), reading.battery()),
        (-29, 100, 0)
    );
}
