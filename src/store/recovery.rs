use super::*;
use anyhow::ensure;
use redb::{BackendError, StorageBackend, backends::FileBackend};
use std::{
    env, fs,
    io::{self, BufRead, Write},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

const POLICY: Policy = Policy {
    high_above: 55,
    high_seconds: 1800,
    recovery_seconds: 300,
    loss_seconds: 600,
};
fn address() -> Address {
    "AA:BB:CC:DD:EE:FF".parse().unwrap()
}
fn reading(humidity: u8) -> Reading {
    let bytes = [
        0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x36, 3, 2, 0x96, humidity, 0,
    ];
    Reading::decode(address(), address(), &[b'w', 0, 100], &bytes).unwrap()
}
fn open(path: &Path) -> Result<Store> {
    Store::open(path, address(), 0, POLICY, 365)
}

#[test]
fn stopped_writer() -> Result<()> {
    let Some(path) = env::var_os("METER_CRASH_TEST_PATH") else {
        return Ok(());
    };
    let mut store = open(Path::new(&path))?;
    let tx = store.db.begin_write()?;
    if env::var("METER_CRASH_TEST_POINT")? == "staged" {
        ensure!(store.stage(&tx, 602, Some(reading(56)))?);
        wait_for_kill()?;
    } else {
        drop(tx);
        store.record(602, reading(56))?;
        wait_for_kill()?;
    }
    Ok(())
}

fn wait_for_kill() -> Result<()> {
    println!("METER_WRITER_READY");
    io::stdout().flush()?;
    loop {
        thread::park();
    }
}

#[test]
fn killed_writer_recovers_an_entire_commit_at_either_boundary() -> Result<()> {
    for (point, committed) in [("staged", false), ("committed", true)] {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("meter.redb");
        open(&path)?.record(1, reading(45))?;
        let mut child = Command::new(env::current_exe()?)
            .args(["--exact", "store::recovery::stopped_writer", "--nocapture"])
            .env("METER_CRASH_TEST_PATH", &path)
            .env("METER_CRASH_TEST_POINT", point)
            .stdout(Stdio::piped())
            .spawn()?;
        let output = child.stdout.take().unwrap();
        let (ready, receiver) = mpsc::channel();
        let reader = thread::spawn(move || {
            if io::BufReader::new(output)
                .lines()
                .map_while(Result::ok)
                .any(|s| s == "METER_WRITER_READY")
            {
                let _ = ready.send(());
            }
        });
        let result = receiver.recv_timeout(Duration::from_secs(15));
        let _ = child.kill();
        child.wait()?;
        reader.join().unwrap();
        result?;
        assert_commit(&path, committed)?;
    }
    Ok(())
}

fn assert_commit(path: &Path, committed: bool) -> Result<()> {
    let mut store = open(path)?;
    let latest = store.latest()?.unwrap();
    let rows = store.history(0, 603, 3)?;
    if committed {
        assert_eq!((latest.received, latest.humidity, rows.len()), (602, 56, 2));
        assert_eq!(rows[0], latest);
        assert_eq!(store.state()?.pending_since, Some(602));
        assert_eq!(store.pending()?, Some((1, Event::Lost, 602)));
        store.acknowledge(1)?;
        assert_eq!(store.pending()?, Some((2, Event::Received, 602)));
    } else {
        assert_eq!((latest.received, latest.humidity, rows.len()), (1, 45, 1));
        assert_eq!(rows[0], latest);
        assert_eq!(store.state()?.pending_since, None);
        assert_eq!(store.pending()?, None);
    }
    Ok(())
}

#[test]
fn an_open_dashboard_snapshot_keeps_one_generation_while_the_writer_advances() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut writer = open(&dir.path().join("meter.redb"))?;
    writer.record(1, reading(45))?;
    let before = writer.snapshot()?;
    let mut next = writer.clone();
    thread::spawn(move || next.record(602, reading(56)))
        .join()
        .unwrap()?;
    assert_eq!(before.latest()?.unwrap().received, 1);
    assert_eq!(before.state()?.received, Some(1));
    assert_eq!(before.history(i64::MIN, i64::MAX, 2)?.len(), 1);
    assert_eq!(before.bins(1, 1)?[0].last_rh, 45);
    assert!(before.bins(2, 1)?.is_empty());
    let after = writer.snapshot()?;
    assert_eq!(after.latest()?.unwrap().received, 602);
    assert_eq!(after.state()?.pending_since, Some(602));
    assert_eq!(after.history(0, 603, 2)?.len(), 2);
    assert_eq!(after.bins(1, 602)?.len(), 2);
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Failure {
    Full,
    Sync,
}
#[derive(Debug)]
struct FaultyFile {
    file: FileBackend,
    fault: Failure,
    armed: Arc<AtomicBool>,
}
impl FaultyFile {
    fn check(&self, kind: Failure) -> std::result::Result<(), BackendError> {
        if self.fault == kind && self.armed.swap(false, Ordering::SeqCst) {
            return Err(BackendError::Io(io::Error::from(
                if kind == Failure::Full {
                    io::ErrorKind::StorageFull
                } else {
                    io::ErrorKind::Other
                },
            )));
        }
        Ok(())
    }
}
impl StorageBackend for FaultyFile {
    fn len(&self) -> std::result::Result<u64, BackendError> {
        self.file.len()
    }
    fn read(&self, offset: u64, out: &mut [u8]) -> std::result::Result<(), BackendError> {
        self.file.read(offset, out)
    }
    fn set_len(&self, len: u64) -> std::result::Result<(), BackendError> {
        self.check(Failure::Full)?;
        self.file.set_len(len)
    }
    fn sync_data(&self) -> std::result::Result<(), BackendError> {
        self.check(Failure::Sync)?;
        self.file.sync_data()
    }
    fn write(&self, offset: u64, data: &[u8]) -> std::result::Result<(), BackendError> {
        self.check(Failure::Full)?;
        self.file.write(offset, data)
    }
}

#[test]
fn a_full_disk_or_failed_sync_never_separates_samples_from_notifications() -> Result<()> {
    for fault in [Failure::Full, Failure::Sync] {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("meter.redb");
        open(&path)?.record(1, reading(45))?;
        let armed = Arc::new(AtomicBool::new(false));
        let file = fs::File::options().read(true).write(true).open(&path)?;
        let db = Database::builder().create_with_backend(FaultyFile {
            file: FileBackend::new(file),
            fault,
            armed: armed.clone(),
        })?;
        let mut store = Store {
            db: Arc::new(db),
            policy: POLICY,
            retention: 365 * 86400,
            notifications: true,
        };
        armed.store(true, Ordering::SeqCst);
        assert!(store.record(602, reading(56)).is_err());
        assert!(!armed.load(Ordering::SeqCst));
        drop(store);
        let recovered = open(&path)?;
        let committed = recovered.latest()?.unwrap().received == 602;
        drop(recovered);
        if fault == Failure::Full {
            assert!(!committed);
        }
        assert_commit(&path, committed)?;
    }
    Ok(())
}
