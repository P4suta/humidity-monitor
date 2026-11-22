use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, ensure};
use jiff::tz::TimeZone;
use serde::Deserialize;
use switchbot_meter::{Address, Policy};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    address: String,
    database: PathBuf,
    label: Option<String>,
    adapter: Option<String>,
    listen: Option<SocketAddr>,
    timezone: Option<String>,
    retention_days: Option<u32>,
    reference: Option<[u8; 2]>,
    dry_below: Option<u8>,
    policy: FilePolicy,
    ntfy: Option<NtfyConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePolicy {
    high_above: u8,
    high_seconds: u32,
    recovery_seconds: u32,
    loss_seconds: u32,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NtfyConfig {
    pub url: String,
    pub token_file: PathBuf,
}

#[derive(Clone)]
pub struct Config {
    pub address: Address,
    pub database: PathBuf,
    pub label: String,
    pub adapter: String,
    pub listen: SocketAddr,
    pub timezone: TimeZone,
    pub retention_days: u32,
    pub reference: Option<[u8; 2]>,
    pub dry_below: Option<u8>,
    pub policy: Policy,
    pub ntfy: Option<NtfyConfig>,
}

pub fn valid_adapter(adapter: &str) -> bool {
    adapter
        .strip_prefix("hci")
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}

impl Config {
    pub fn parse(input: &str) -> Result<Self> {
        let f: FileConfig = toml::from_str(input).context("invalid monitor configuration")?;
        crate::store::require_database_file(&f.database)?;
        let p = f.policy;
        ensure!(
            p.high_above <= 100
                && p.high_seconds > 0
                && p.recovery_seconds > 0
                && p.loss_seconds > 0,
            "invalid monitoring policy"
        );
        ensure!(
            f.reference.is_none_or(|[lo, hi]| lo < hi && hi <= 100)
                && f.dry_below.is_none_or(|v| v <= 100),
            "invalid display range"
        );
        let retention_days = f.retention_days.unwrap_or(365);
        ensure!(
            retention_days > 0,
            "retention must include at least one day"
        );
        let adapter = f.adapter.unwrap_or_else(|| "hci0".into());
        ensure!(valid_adapter(&adapter), "invalid BlueZ adapter");
        let label = f.label.unwrap_or_else(|| "SwitchBot".into());
        ensure!(
            !label.trim().is_empty() && !label.chars().any(char::is_control),
            "invalid display label"
        );
        Ok(Self {
            address: f.address.parse().map_err(anyhow::Error::msg)?,
            database: f.database,
            label,
            adapter,
            listen: f.listen.unwrap_or(([127, 0, 0, 1], 8080).into()),
            timezone: TimeZone::get(f.timezone.as_deref().unwrap_or("UTC"))
                .context("invalid time zone")?,
            retention_days,
            reference: f.reference,
            dry_below: f.dry_below,
            policy: Policy {
                high_above: p.high_above,
                high_seconds: p.high_seconds.into(),
                recovery_seconds: p.recovery_seconds.into(),
                loss_seconds: p.loss_seconds.into(),
            },
            ntfy: f.ntfy,
        })
    }
}
