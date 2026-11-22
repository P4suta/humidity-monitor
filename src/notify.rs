use anyhow::{Result, ensure};
use reqwest::{Client, Url, redirect};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use switchbot_meter::Event;

pub struct Publisher {
    client: Client,
    url: Url,
    token_file: PathBuf,
    label: String,
}

impl Publisher {
    pub fn new(url: &str, token_file: &Path, label: &str) -> Result<Self> {
        let url = Url::parse(url)?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.path().trim_matches('/').split('/').count() == 1
                && !url.path().trim_matches('/').is_empty(),
            "expected an ntfy topic URL"
        );
        let client = Client::builder()
            .redirect(redirect::Policy::none())
            .timeout(Duration::from_secs(8))
            .build()?;
        Ok(Self {
            client,
            url,
            token_file: token_file.into(),
            label: label.into(),
        })
    }

    pub async fn send(&self, event: Event, at: i64) -> Result<()> {
        #[derive(Deserialize)]
        struct Receipt {
            id: String,
            event: String,
        }
        let token = std::fs::read_to_string(&self.token_file)?;
        ensure!(!token.trim().is_empty(), "missing ntfy publisher token");
        let text = match event {
            Event::High => "高湿度",
            Event::Normal => "高湿度から復旧",
            Event::Lost => "受信途絶",
            Event::Received => "受信再開",
        };
        let time = jiff::Timestamp::from_second(at)
            .map(|t| t.to_string())
            .unwrap_or_else(|_| format!("Unix {at}"));
        let response = self
            .client
            .post(self.url.clone())
            .bearer_auth(token.trim())
            .body(format!("{}　{text}\n{time}", self.label))
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;
        ensure!(
            response.status().is_success(),
            "ntfy returned HTTP {}",
            response.status().as_u16()
        );
        let receipt: Receipt = response.json().await.map_err(reqwest::Error::without_url)?;
        ensure!(
            receipt.event == "message" && !receipt.id.is_empty(),
            "ntfy did not confirm the message"
        );
        Ok(())
    }
}
