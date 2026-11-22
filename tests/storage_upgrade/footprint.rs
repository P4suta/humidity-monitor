use super::*;
use anyhow::{Context, ensure};
use std::{
    fs,
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, Instant},
};

fn release() -> Result<PathBuf> {
    Ok(std::env::var_os("HUMIDITY_MONITOR_RELEASE")
        .context("run with just receiver-footprint")?
        .into())
}

struct Receiver(Child);

impl Drop for Receiver {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Receiver {
    async fn start(directory: &Path) -> Result<(Self, SocketAddr)> {
        let addr = TcpListener::bind("127.0.0.1:0")?.local_addr()?;
        let config = format!(
            "listen='{addr}'\naddress='{ADDRESS}'\ndatabase='meter.redb'\n\
             [policy]\nhigh_above=80\nhigh_seconds=300\nrecovery_seconds=60\nloss_seconds=600\n\
             [ntfy]\nurl='http://127.0.0.1:1/test-topic'\ntoken_file='fixture-token'"
        );
        fs::write(directory.join("monitor.toml"), config)?;
        let mut receiver = Self(
            Command::new(release()?)
                .current_dir(directory)
                .env("TOKIO_WORKER_THREADS", "8")
                .env(
                    "DBUS_SYSTEM_BUS_ADDRESS",
                    "unix:path=/nonexistent-meter-bus",
                )
                .args(["serve", "monitor.toml"])
                .spawn()?,
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        while TcpStream::connect(addr).is_err() {
            ensure!(
                receiver.0.try_wait()?.is_none(),
                "receiver exited before listening"
            );
            ensure!(
                Instant::now() < deadline,
                "receiver did not open its listener"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok((receiver, addr))
    }

    fn status(&self, name: &str) -> Result<u64> {
        let status = fs::read_to_string(format!("/proc/{}/status", self.0.id()))?;
        let number = status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|value| value.split_whitespace().next())
            .with_context(|| format!("missing {name} in receiver status"))?;
        Ok(number.parse()?)
    }
}

#[test]
#[ignore = "run with just receiver-footprint"]
fn shipped_binary_is_position_independent_and_statically_linked() -> Result<()> {
    let result = Command::new("file")
        .env("LC_ALL", "C")
        .arg("-b")
        .arg(release()?)
        .output()?;
    let description = String::from_utf8(result.stdout)?;
    ensure!(
        result.status.success() && description.contains("static-pie linked"),
        "{description}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "run with just receiver-footprint"]
async fn waiting_receiver_stays_below_six_mib_with_one_async_worker() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (receiver, _) = Receiver::start(dir.path()).await?;
    let peak = receiver.status("VmHWM:")?;
    let threads = receiver.status("Threads:")?;
    println!("Waiting receiver: {peak} KiB peak RSS, {threads} threads");
    ensure!(peak <= 6 * 1024, "waiting receiver peaked at {peak} KiB");
    ensure!(threads == 2, "waiting receiver started {threads} threads");
    Ok(())
}

#[tokio::test]
#[ignore = "run with just receiver-footprint"]
async fn serving_a_dense_year_stays_below_seven_mib() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let source = legacy(&dir.path().join("meter.sqlite"))?;
    let last = (humidity_monitor::unix_now()? / 60 - 1) * 60;
    source.execute_batch("DELETE FROM samples; DELETE FROM outbox;")?;
    source.execute(
        "WITH RECURSIVE minutes(i) AS (VALUES(0) UNION ALL SELECT i+1 FROM minutes WHERE i+1 < 524160)
         INSERT INTO samples SELECT (?1-i*60)/60, ?1-i*60, 200, i%101, 99 FROM minutes",
        [last],
    )?;
    source.execute(
        "UPDATE monitor SET received=?1, humidity=0, pending_since=NULL",
        [last],
    )?;
    drop(source);
    drop(open(&dir.path().join("meter.redb"))?);

    let (receiver, addr) = Receiver::start(dir.path()).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    for _ in 0..3 {
        let html = client
            .get(format!("http://{addr}/?period=1y&paused=true"))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        ensure!(
            html.contains("以前の測定値") && html.contains("class=\"series\""),
            "expected a year of recorded history"
        );
    }
    let peak = receiver.status("VmHWM:")?;
    println!("Annual dashboard: {peak} KiB peak receiver RSS");
    ensure!(peak <= 7 * 1024, "annual dashboard peaked at {peak} KiB");
    Ok(())
}
