use crate::{Config, Store, trend::Bin, unix_now};
use anyhow::Result;
use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::Html,
    routing::get,
};
use maud::{DOCTYPE, Markup, html};
use serde::Deserialize;
use std::{
    fmt::Write,
    sync::{Arc, Mutex},
};

#[derive(Copy, Clone, Default, Deserialize, PartialEq, Eq)]
enum Period {
    #[default]
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
    #[serde(rename = "1y")]
    Year,
}
impl Period {
    const ALL: [Self; 4] = [Self::Day, Self::Week, Self::Month, Self::Year];
    fn info(self) -> (&'static str, &'static str, i64) {
        match self {
            Self::Day => ("24h", "24時間", 1),
            Self::Week => ("7d", "7日", 7),
            Self::Month => ("30d", "30日", 30),
            Self::Year => ("1y", "1年", 365),
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct View {
    #[serde(default)]
    period: Period,
    before: Option<i64>,
    paused: Option<bool>,
    #[serde(default)]
    history: bool,
}
#[derive(Clone)]
struct App {
    store: Arc<Mutex<Store>>,
    config: Arc<Config>,
}

pub fn dashboard(store: Arc<Mutex<Store>>, config: Arc<Config>) -> Router {
    Router::new()
        .route("/", get(index))
        .with_state(App { store, config })
}
async fn index(
    State(app): State<App>,
    Query(view): Query<View>,
) -> Result<Html<String>, StatusCode> {
    tokio::task::spawn_blocking(move || page(&app, view))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(|m| Html(m.into_string()))
        .map_err(|e| {
            eprintln!("history query failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}
fn date(at: i64, config: &Config) -> String {
    jiff::Timestamp::from_second(at)
        .map(|t| {
            t.to_zoned(config.timezone.clone())
                .strftime("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| format!("Unix {at}"))
}
fn time(at: i64, config: &Config) -> Markup {
    let utc = jiff::Timestamp::from_second(at)
        .ok()
        .map(|timestamp| timestamp.to_string());
    html! { time datetime=[utc] {(date(at,config))} }
}
fn coordinate(at: i64, start: i64, end: i64, rh: u8) -> (f64, f64) {
    (
        32.0 + 532.0 * (at - start) as f64 / (end - start) as f64,
        118.0 - f64::from(rh),
    )
}

fn series(run: &[Bin], start: i64, end: i64) -> Option<String> {
    if matches!(run, [only] if only.first == only.last) {
        return None;
    }
    let mut line = String::new();
    for b in run {
        let op = if line.is_empty() { "M" } else { "L" };
        let (x, y) = coordinate(b.first.at, start, end, b.first.rh);
        write!(line, "{op}{x:.2},{y:.1} ").unwrap();
        if b.first != b.last {
            let (x, y) = coordinate(b.last.at, start, end, b.last.rh);
            write!(line, "L{x:.2},{y:.1} ").unwrap();
        }
    }
    Some(line)
}

fn chart(data: &[Bin], start: i64, end: i64, config: &Config) -> Markup {
    let runs = data.chunk_by(|a, b| a.last.at / 60 + 1 == b.first.at / 60);
    html! {figure{
        svg viewBox="0 0 580 136" role="img" aria-labelledby="plot-title plot-desc" {
            title id="plot-title" {"相対湿度の推移"}
            desc id="plot-desc" {
                "縦軸は相対湿度、横軸は受信時刻です。"
                "欠測区間で線が途切れます。"
                "欠測が多い区間では途中の最小値と最大値を点で示します。"
                "測定値は履歴表で確認できます。"
            }
            @if let Some([lo,hi])=config.reference {rect.reference x="32" y=(118-hi) width="532" height=(hi-lo) {}}
            @for tick in [0,50,100]{line.grid x1="32" x2="564" y1=(118-tick) y2=(118-tick){} text x="26" y=(122-tick) text-anchor="end"{(tick)}}
            @for run in runs { @if let Some(line) = series(run, start, end) {path.series d=(line) {}} }
            @for b in data{
                @let (x,y)=coordinate(b.first.at,start,end,b.range.low.rh);
                @let (_,v)=coordinate(b.first.at,start,end,b.range.high.rh);
                @if b.range.low.rh!=b.range.high.rh {line.extent x1=(x) x2=(x) y1=(y) y2=(v) {}}
                circle cx=(x) cy=(y) r="1.8"{}
                @if b.range.low.rh!=b.range.high.rh{circle cx=(x) cy=(v) r="1.8"{}}
            }
        }
        figcaption {span{(date(start,config))} span{(date(end,config))}}
        @if let Some([lo,hi])=config.reference {p.legend{"網掛けは目安の " (lo) "〜" (hi) "％です。"}}
        @if data.is_empty(){p{"期間内の記録はありません。"}}
    }}
}

fn page(app: &App, view: View) -> Result<Markup> {
    const PAGE_SIZE: usize = 12;
    let now = unix_now()?;
    let c = &app.config;
    let (key, label, days) = view.period.info();
    let start = now - days * 86400;
    let db = app
        .store
        .lock()
        .map_err(|_| anyhow::anyhow!("database unavailable"))?
        .snapshot()?;
    let current = db.latest()?;
    let state = db.state()?;
    let data = db.bins(start, now)?;
    let end = view.before.unwrap_or(now + 1).clamp(start, now + 1);
    let rows = db.history(start, end, PAGE_SIZE as u32 + 1)?;
    drop(db);
    let stale = now - state.received.unwrap_or(state.started) >= c.policy.loss_seconds;
    let history = view.history || view.before.is_some();
    let paused = view.paused.unwrap_or(false);
    Ok(html! {(DOCTYPE) html lang="ja"{head{
        meta charset="utf-8"; meta name="viewport" content="width=device-width, initial-scale=1";
        @if !paused && !history {meta http-equiv="refresh" content="60";}
        title{(c.label) " · 温湿度"} style{(maud::PreEscaped(include_str!("dashboard.css")))}
    }body{main{
        section id="current" aria-labelledby="meter"{
            h1 id="meter"{(c.label)}
            p.metric{span.caption{"相対湿度"} @if let Some(r)=&current{(r.humidity) small{"％"}}@else{"—"}}
            @if stale {p.status{"受信途絶"}} @else if current.is_none(){p.status{"受信待ち"}}
            @if let Some(r)=&current {
                p{"最終受信 " (time(r.received,c)) " " (c.timezone.iana_name().unwrap_or("UTC"))}
                p.detail{(format!("{:.1}",f64::from(r.tenths_c)/10.0)) " ℃　電池 " (r.battery) "％"
                    @if c.dry_below.is_some_and(|limit|r.humidity<limit){"　乾燥"}
                    @if state.high{"　高湿度"}
                }
            }@else{p{"有効な広告の受信を待っています。"}}
        }
        section aria-labelledby="trend"{h2 id="trend"{"湿度の推移"}
            nav aria-label="表示期間"{@for period in Period::ALL{
                @let (slug,name,_)=period.info();
                a href=(format!("/?period={slug}&paused={paused}")) data-period=(slug) aria-current=[(period==view.period).then_some("page")]{(name)}
            }}
            (chart(&data,start,now,c))
        }
        section id="history" aria-label="測定履歴"{
            @if !history {
                a.history-link href=(format!("/?period={key}&paused={paused}&history=true#history")) {"測定履歴を見る"}
            }
            div hidden[!history] {
            h2{"測定履歴"}
            table{caption{(label) "の測定値 · " (c.timezone.iana_name().unwrap_or("UTC"))}
                thead{tr{th scope="col"{"受信時刻"}th scope="col"{"相対湿度"}}}
                tbody{@for (i,r) in rows.iter().take(PAGE_SIZE).enumerate(){
                    @if i==0 && end/60-r.received/60>1 || i>0 && rows[i-1].received/60-r.received/60>1 {tr.gap{td colspan="2"{"欠測"}}}
                    tr{td{(time(r.received,c))}td{(r.humidity)"％"}}
                }}
            }
            nav.history-pages aria-label="履歴の移動" {
                @if rows.len()>PAGE_SIZE {a.more href=(format!("/?period={key}&paused={paused}&before={}#history",rows[PAGE_SIZE-1].received)){"以前の測定値"}}
                @if view.before.is_some() {a href=(format!("/?period={key}&paused={paused}&history=true#history")){"最新の測定値"}}
                a href=(format!("/?period={key}&paused={paused}#trend")){"グラフに戻る"}
            }
            }
        }
        @if !history {footer{a href=(format!("/?period={key}&paused={paused}")){"更新"}
            a href=(format!("/?period={key}&paused={}",!paused)){@if paused{"自動更新を再開"}@else{"自動更新を停止"}}
        }}
    }}} })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Sample, trend};

    #[test]
    fn series_draws_each_run_endpoint_and_leaves_isolated_points_to_the_markers() {
        let rows = [50, 52, 46, 47, 51]
            .into_iter()
            .enumerate()
            .map(|(minute, humidity)| {
                Ok::<_, ()>(Sample {
                    received: minute as i64 * 60,
                    humidity,
                    tenths_c: 200,
                    battery: 99,
                })
            });
        let end = 532 * 60;
        let bins = trend::group(rows, 0, end).unwrap();
        assert_eq!(
            series(&bins, 0, end).as_deref().map(str::trim_end),
            Some("M32.00,68.0 L33.00,66.0 L34.00,72.0 L35.00,71.0 L36.00,67.0"),
        );
        assert!(series(&bins[2..], 0, end).is_none());
    }
}
