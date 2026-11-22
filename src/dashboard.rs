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
use rusqlite::params;
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
        .unwrap_or_default()
}
fn coordinate(at: i64, start: i64, end: i64, rh: u8) -> (f64, f64) {
    (
        32.0 + 532.0 * (at - start) as f64 / (end - start) as f64,
        118.0 - f64::from(rh),
    )
}
fn bins(store: &Store, start: i64, end: i64) -> Result<Vec<Bin>> {
    let mut query = store.db.prepare(
        "SELECT received,temperature,humidity,battery FROM samples
         WHERE minute BETWEEN ?1/60 AND ?2/60 AND received BETWEEN ?1 AND ?2 ORDER BY minute",
    )?;
    let rows = query.query_map(params![start, end], crate::Sample::row)?;
    Ok(crate::trend::group(rows, start, end)?)
}

fn chart(data: &[Bin], start: i64, end: i64, config: &Config) -> Markup {
    let mut runs = Vec::new();
    let mut line = String::new();
    let mut previous = None;
    for b in data {
        if previous != Some(b.first / 60 - 1) && !line.is_empty() {
            runs.push(std::mem::take(&mut line));
        }
        let op = if line.is_empty() { "M" } else { "L" };
        let (x, y) = coordinate(b.first, start, end, b.first_rh);
        let (u, v) = coordinate(b.last, start, end, b.last_rh);
        write!(line, "{op}{x:.2},{y:.1} L{u:.2},{v:.1} ").unwrap();
        previous = Some(b.last / 60);
    }
    if !line.is_empty() {
        runs.push(line);
    }
    html! {figure{
        svg viewBox="0 0 580 136" role="img" aria-labelledby="plot-title plot-desc" {
            title id="plot-title" {"相対湿度の推移"}
            desc id="plot-desc" {"縦軸は相対湿度、横軸は受信時刻です。欠測区間で線が途切れます。測定値は履歴表で確認できます。"}
            @if let Some([lo,hi])=config.reference {rect.reference x="32" y=(118-hi) width="532" height=(hi-lo) {}}
            @for tick in [0,50,100]{line.grid x1="32" x2="564" y1=(118-tick) y2=(118-tick){} text x="26" y=(122-tick) text-anchor="end"{(tick)}}
            @for run in runs{path.series d=(run) {}}
            @for b in data{
                @let (x,y)=coordinate(b.first,start,end,b.low);
                @let (_,v)=coordinate(b.first,start,end,b.high);
                @if b.low!=b.high {line.extent x1=(x) x2=(x) y1=(y) y2=(v) {}}
                circle cx=(x) cy=(y) r="1.8"{}
                @if b.low!=b.high{circle cx=(x) cy=(v) r="1.8"{}}
            }
        }
        figcaption {span{(date(start,config))} span{(date(end,config))}}
        @if let Some([lo,hi])=config.reference {p.legend{"網掛けは目安の " (lo) "〜" (hi) "％です。"}}
        @if data.is_empty(){p{"期間内の記録はありません。"}}
    }}
}

fn page(app: &App, view: View) -> Result<Markup> {
    let now = unix_now()?;
    let c = &app.config;
    let (key, label, days) = view.period.info();
    let start = now - days * 86400;
    let db = app
        .store
        .lock()
        .map_err(|_| anyhow::anyhow!("database unavailable"))?;
    let current = db.latest()?;
    let state = db.state()?;
    let data = bins(&db, start, now)?;
    let end = view.before.unwrap_or(now + 1).clamp(start, now + 1);
    let rows = db.history(start, end, 61)?;
    drop(db);
    let stale = now - state.received.unwrap_or(state.started) >= c.policy.loss_seconds;
    let paused = view.paused.unwrap_or(false) || view.before.is_some();
    Ok(html! {(DOCTYPE) html lang="ja"{head{
        meta charset="utf-8"; meta name="viewport" content="width=device-width, initial-scale=1";
        @if !paused {meta http-equiv="refresh" content="60";}
        title{(c.label) " · 温湿度"} style{(maud::PreEscaped(include_str!("dashboard.css")))}
    }body{main{
        section id="current" aria-labelledby="meter"{
            h1 id="meter"{(c.label)}
            p.metric{span.caption{"相対湿度"} @if let Some(r)=&current{(r.humidity) small{"％"}}@else{"—"}}
            @if stale {p.status{"受信途絶"}} @else if current.is_none(){p.status{"受信待ち"}}
            @if let Some(r)=&current {
                p{"最終受信 " time{(date(r.received,c))} " " (c.timezone.iana_name().unwrap_or("UTC"))}
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
        section aria-labelledby="history"{h2 id="history"{"測定履歴"}
            table{caption{(label) "の測定値 · " (c.timezone.iana_name().unwrap_or("UTC"))}
                thead{tr{th scope="col"{"受信時刻"}th scope="col"{"相対湿度"}}}
                tbody{@for (i,r) in rows.iter().take(60).enumerate(){
                    @if i==0 && end/60-r.received/60>1 || i>0 && rows[i-1].received/60-r.received/60>1 {tr.gap{td colspan="2"{"欠測"}}}
                    tr{td{(date(r.received,c))}td{(r.humidity)"％"}}
                }}
            }
            @if rows.len()>60{a.more href=(format!("/?period={key}&before={}",rows[59].received)){"以前の測定値"}}
        }
        footer{a href=(format!("/?period={key}&paused={paused}")){"更新"}
            a href=(format!("/?period={key}&paused={}",!paused)){@if paused{"自動更新を再開"}@else{"自動更新を停止"}}
        }
    }}} })
}
