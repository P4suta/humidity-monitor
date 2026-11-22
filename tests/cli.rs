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

#[cfg(unix)]
#[test]
fn the_service_exits_cleanly_on_a_container_stop_signal() -> anyhow::Result<()> {
    use std::{
        net::{TcpListener, TcpStream},
        process::Stdio,
        time::{Duration, Instant},
    };
    let directory = tempfile::tempdir()?;
    let address = TcpListener::bind("127.0.0.1:0")?.local_addr()?;
    let config = format!(
        "listen='{address}'\naddress='AA:BB:CC:DD:EE:FF'\ndatabase='meter.sqlite'\n[policy]\nhigh_above=70\nhigh_seconds=60\nrecovery_seconds=10\nloss_seconds=60"
    );
    std::fs::write(directory.path().join("monitor.toml"), config)?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_humidity-monitor"))
        .current_dir(directory.path())
        .args(["serve", "monitor.toml"])
        .stderr(Stdio::null())
        .spawn()?;
    let start = Instant::now();
    while TcpStream::connect(address).is_err() {
        if start.elapsed() > Duration::from_secs(8) || child.try_wait()?.is_some() {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("receiver did not open its listener");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let signal = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()?;
    assert!(signal.success());
    loop {
        if let Some(status) = child.try_wait()? {
            assert!(status.success(), "service stop failed with {status}");
            break;
        }
        if start.elapsed() > Duration::from_secs(10) {
            child.kill()?;
            child.wait()?;
            anyhow::bail!("receiver did not stop on SIGTERM");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}
