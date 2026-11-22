pub mod bluez;
mod config;
mod dashboard;
mod notify;
mod store;
pub use config::{Config, valid_adapter};
pub use dashboard::dashboard;
pub use notify::Publisher;
pub use store::{Sample, Store};

pub fn unix_now() -> anyhow::Result<i64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        .try_into()?)
}
