use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use humidity_monitor::{Config, Publisher, Store, bluez, dashboard, unix_now, valid_adapter};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    time::{MissedTickBehavior, interval, sleep},
};

#[derive(Parser)]
#[command(version, about = "Read SwitchBot outdoor meters using BlueZ")]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// List nearby outdoor meters with complete readings.
    Discover {
        #[arg(default_value = "hci0")]
        adapter: String,
    },
    /// Record a registered meter and serve its humidity history.
    Serve { config: PathBuf },
}

#[tokio::main(worker_threads = 1)]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Action::Discover { adapter } => {
            ensure!(valid_adapter(&adapter), "invalid adapter name");
            let bus = zbus::Connection::system()
                .await
                .context("open system bus")?;
            let mut seen = HashSet::new();
            tokio::select! {
                result=bluez::scan(&bus,&adapter,None,|address,reading|{
                    if seen.insert(address){println!("{address}  {:.1} °C  {}% RH  {}% battery",f64::from(reading.tenths_c())/10.0,reading.humidity(),reading.battery());}
                    Ok(())
                })=>result,
                result=tokio::signal::ctrl_c()=>Ok(result?),
            }
        }
        Action::Serve { config } => {
            serve(Config::parse(
                &std::fs::read_to_string(config).context("read configuration")?,
            )?)
            .await
        }
    }
}

type Database = Arc<Mutex<Store>>;

fn lock(db: &Database) -> Result<std::sync::MutexGuard<'_, Store>> {
    db.lock()
        .map_err(|_| anyhow::anyhow!("database lock poisoned"))
}

async fn serve(config: Config) -> Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let now = unix_now()?;
    let store = Store::open(
        &config.database,
        config.address,
        now,
        config.policy,
        config.retention_days,
    )?;
    let reader = Arc::new(Mutex::new(store.clone()));
    let writer = Arc::new(Mutex::new(store.with_notifications(config.ntfy.is_some())));
    let publisher = config
        .ntfy
        .as_ref()
        .map(|n| Publisher::new(&n.url, &n.token_file, &config.label))
        .transpose()?;
    let config = Arc::new(config);
    let listener = TcpListener::bind(config.listen)
        .await
        .context("bind HTTP listener")?;
    let app = dashboard(reader, config.clone());
    tokio::select! {
        result=axum::serve(listener,app).into_future()=>Ok(result?),
        result=collect(&config,&writer)=>result,
        result=timers(&writer)=>result,
        result=deliver(&writer,publisher)=>result,
        _=terminate.recv()=>Ok(()),
        result=tokio::signal::ctrl_c()=>Ok(result?),
    }
}

async fn collect(config: &Config, db: &Database) -> Result<()> {
    loop {
        let result = async {
            let bus = zbus::Connection::system().await?;
            bluez::scan(&bus, &config.adapter, Some(config.address), |_, reading| {
                lock(db)?.record(unix_now()?, reading)
            })
            .await
        }
        .await;
        if let Err(error) = result {
            eprintln!("Bluetooth collection interrupted: {error:#}");
        }
        sleep(Duration::from_secs(5)).await;
    }
}

async fn timers(db: &Database) -> Result<()> {
    let mut clock = interval(Duration::from_secs(1));
    clock.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        clock.tick().await;
        lock(db)?.tick(unix_now()?)?;
    }
}

async fn deliver(db: &Database, publisher: Option<Publisher>) -> Result<()> {
    let Some(publisher) = publisher else {
        return std::future::pending().await;
    };
    loop {
        let pending = lock(db)?.pending()?;
        if let Some((id, event, at)) = pending {
            if let Err(error) = publisher.send(event, at).await {
                eprintln!("Notification {id} remains pending: {error}");
                sleep(Duration::from_secs(15)).await;
                continue;
            }
            lock(db)?.acknowledge(id)?;
        } else {
            sleep(Duration::from_secs(1)).await;
        }
    }
}
