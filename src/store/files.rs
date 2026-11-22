use super::{legacy, schema::*};
use anyhow::{Context, Result, ensure};
use redb::{Database, ReadableDatabase, ReadableTable};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{ErrorKind, Read},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};
use switchbot_meter::Address;

fn entry_exists(path: &Path) -> std::io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn sqlite_file(path: &Path) -> Result<bool> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    ensure!(metadata.is_file(), "database path must name a regular file");
    if metadata.len() < 16 {
        return Ok(false);
    }
    let mut magic = [0; 16];
    File::open(path)?.read_exact(&mut magic)?;
    Ok(&magic == b"SQLite format 3\0")
}

fn paths(path: &Path) -> Result<(PathBuf, Option<PathBuf>)> {
    if path.extension() == Some(OsStr::new("sqlite")) {
        return Ok((path.with_extension("redb"), Some(path.to_owned())));
    }
    let companion = path.with_added_extension("redb");
    if sqlite_file(path)? || (!entry_exists(path)? && entry_exists(&companion)?) {
        return Ok((companion, Some(path.to_owned())));
    }
    let source =
        (path.extension() == Some(OsStr::new("redb"))).then(|| path.with_extension("sqlite"));
    Ok((path.to_owned(), source))
}

const CACHE_BYTES: usize = 16 * 1024 * 1024;

fn open_existing(path: &Path, address: Address) -> Result<Database> {
    ensure!(
        fs::metadata(path)?.is_file(),
        "database path must name a regular file"
    );
    let db = Database::builder()
        .set_cache_size(CACHE_BYTES)
        .set_check_integrity_on_open(true)
        .open(path)
        .with_context(|| format!("open redb history at {}", path.display()))?;
    {
        let tx = db.begin_read()?;
        let identities = tx.open_table(IDENTITY)?;
        let meter = identities.get(())?.context("missing meter identity")?;
        let (format, stored) = meter.value();
        ensure!(
            format == FORMAT,
            "unsupported meter history format {format}"
        );
        ensure!(
            stored == address.to_string(),
            "database belongs to a different meter"
        );
        ensure!(
            tx.open_table(STATE)?.get(())?.is_some(),
            "missing receiver state"
        );
        tx.open_table(LATEST)?;
        tx.open_table(SAMPLES)?;
        tx.open_table(OUTBOX)?;
    }
    Ok(db)
}

pub(super) fn open(path: &Path, address: Address, now: i64) -> Result<Database> {
    open_with_directory_sync(path, address, now, File::sync_all)
}

fn open_with_directory_sync(
    path: &Path,
    address: Address,
    now: i64,
    sync_directory: impl FnOnce(&File) -> std::io::Result<()>,
) -> Result<Database> {
    let (target, source) = paths(path)?;
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let directory = File::open(parent).context("open database directory")?;
    let open_target = || {
        sync_directory(&directory).context("sync meter history directory")?;
        open_existing(&target, address)
    };
    if entry_exists(&target)? {
        return open_target();
    }
    directory
        .lock()
        .context("lock database directory during initialization")?;
    if entry_exists(&target)? {
        return open_target();
    }
    let workspace = target.with_added_extension("humidity-stage");
    match fs::DirBuilder::new().mode(0o700).create(&workspace) {
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            ensure!(
                fs::symlink_metadata(&workspace)?.is_dir(),
                "invalid migration workspace"
            );
        }
        result => result?,
    }
    let pending = workspace.join("partial");
    match fs::remove_file(&pending) {
        Err(e) if e.kind() == ErrorKind::NotFound => (),
        result => result?,
    }
    let staged = tempfile::Builder::new()
        .prefix("partial")
        .rand_bytes(0)
        .tempfile_in(&workspace)?;
    let result = (|| -> Result<()> {
        let db = Database::builder()
            .set_cache_size(CACHE_BYTES)
            .create(staged.path())?;
        let tx = db.begin_write()?;
        tx.open_table(IDENTITY)?
            .insert((), (FORMAT, address.to_string().as_str()))?;
        tx.open_table(LATEST)?;
        tx.open_table(OUTBOX)?;
        tx.open_table(SAMPLES)?;
        if let Some(source) = source
            && entry_exists(&source)?
        {
            legacy::import(&source, &tx, address).context("import SQLite meter history")?;
        } else {
            tx.open_table(STATE)?
                .insert((), (now, None, false, false, 0))?;
        }
        tx.commit()?;
        drop(db);
        staged.as_file().sync_all()?;
        staged.persist_noclobber(&target)?;
        Ok(())
    })();
    if result.is_ok() {
        fs::remove_dir(&workspace)?;
    } else {
        let _ = fs::remove_dir(&workspace);
    }
    result?;
    open_target()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_published_name_requires_a_successful_directory_sync_on_every_open() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("meter.redb");
        let address = "AA:BB:CC:DD:EE:FF".parse().unwrap();
        for now in [41, 42] {
            let error = open_with_directory_sync(&path, address, now, |directory| {
                assert!(directory.metadata()?.is_dir());
                Err(std::io::Error::other("injected directory sync failure"))
            })
            .expect_err("accepted readings without a durable directory entry");
            assert_eq!(
                error.root_cause().to_string(),
                "injected directory sync failure"
            );
            assert!(path.is_file());
        }
        let db = open(&path, address, 43)?;
        let tx = db.begin_read()?;
        assert_eq!(tx.open_table(STATE)?.get(())?.unwrap().value().0, 41);
        Ok(())
    }
}
