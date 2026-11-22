use std::process::Command;

#[test]
fn cli_lists_discovery_and_rejects_an_invalid_adapter_before_opening_dbus() {
    let program = env!("CARGO_BIN_EXE_humidity-monitor");
    let help = Command::new(program).arg("--help").output().unwrap();
    assert!(help.status.success());
    let text = String::from_utf8(help.stdout).unwrap();
    assert!(text.contains("discover") && text.contains("serve"));
    let invalid = Command::new(program)
        .args(["discover", "../hci0"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("adapter"));
}
