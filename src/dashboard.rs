mod curve;

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
use std::sync::{Arc, Mutex};

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
fn humidity_bounds(observed: Option<[u8; 2]>, reference: Option<[u8; 2]>) -> [u8; 2] {
    let Some([low, high]) = observed
        .into_iter()
        .chain(reference)
        .reduce(|[a, b], [c, d]| [a.min(c), b.max(d)])
    else {
        return [0, 100];
    };
    [
        low.saturating_sub(2) / 10 * 10,
        (high + 2).min(100).div_ceil(10) * 10,
    ]
}

fn axis_time(at: i64, format: &str, config: &Config) -> Markup {
    let Ok(stamp) = jiff::Timestamp::from_second(at) else {
        return time(at, config);
    };
    let local = stamp.to_zoned(config.timezone.clone());
    html! { time datetime=(stamp) title=(date(at,config)) { (local.strftime(format)) } }
}

fn chart(data: &[Bin], start: i64, end: i64, config: &Config) -> Markup {
    let observed = data
        .iter()
        .map(|b| [b.range.low.rh, b.range.high.rh])
        .reduce(|[a, b], [c, d]| [a.min(c), b.max(d)]);
    let [low, high] = humidity_bounds(observed, config.reference);
    let y = |humidity: u8| 178.0 - 158.0 * f64::from(humidity - low) / f64::from(high - low);
    let x = |at: i64| 48.0 + 516.0 * (at - start) as f64 / (end - start) as f64;
    let runs = data.chunk_by(|a, b| a.last.at / 60 + 1 == b.first.at / 60);
    html! {figure{
        svg viewBox="0 0 580 190" role="img" aria-labelledby="plot-title plot-desc" {
            title id="plot-title" {"相対湿度の推移"}
            desc id="plot-desc" {
                "縦軸は相対湿度、横軸は受信時刻です。"
                "曲線は連続して記録した湿度を結びます。欠測区間で線が途切れます。"
                "点は単独の測定値です。"
            }
            @if let Some([lo,hi])=config.reference {
                defs {pattern id="reference-hatch" width="12" height="12" patternUnits="userSpaceOnUse" {
                    path.hatch d="M-3 3L3 -3M0 12L12 0M9 15L15 9" { }
                }}
                rect.reference x="48" y=(y(hi)) width="516" height=(y(lo)-y(hi)) fill="url(#reference-hatch)" {}
            }
            @for tick in [high,low+(high-low)/2,low]{
                line.grid x1="48" x2="564" y1=(y(tick)) y2=(y(tick)){}
                text x="40" y=(y(tick)) dominant-baseline="middle" text-anchor="end"{(tick) "%"}
            }
            @for run in runs {
                @if let [bin] = run && bin.first == bin.last {
                    circle cx=(x(bin.first.at)) cy=(y(bin.first.rh)) r="2.8" {}
                } @else {
                    @let points: Vec<_> = curve::anchors(run).into_iter()
                        .map(|p| (x(p.at), y(p.rh))).collect();
                    path.series d=(curve::path(&points)) {}
                }
            }
        }
        div.axis-time {
            @let format = if end-start>86400 { "%m/%d" } else { "%H:%M" };
            @let ends = if end-start>=12*3600 && end-start<=86400 { "%m/%d %H:%M" } else { format };
            (axis_time(start, ends, config))
            (axis_time(start+(end-start)/2, format, config))
            (axis_time(end, ends, config))
        }
        figcaption {
            @if let Some([lo,hi])=observed {span{"最低 " strong{(lo) "％"}} span{"最高 " strong{(hi) "％"}}}
            @if let Some([lo,hi])=config.reference {span{"網掛けの目安 " (lo) "〜" (hi) "％"}}
        }
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
    let installation = state.started.clamp(start, now - 60);
    let plot_start = if db.history(start, installation, 1)?.is_empty() {
        installation
    } else {
        start
    };
    let data = db.bins(plot_start, now)?;
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
            nav.periods aria-label="表示期間"{@for period in Period::ALL{
                @let (slug,name,_)=period.info();
                a href=(format!("/?period={slug}&paused={paused}")) data-period=(slug) aria-current=[(period==view.period).then_some("page")]{(name)}
            }}
            (chart(&data,plot_start,now,c))
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
    fn fits_observations_and_reference_inside_the_humidity_axis() {
        for (observed, reference, expected) in [
            (Some([33, 34]), Some([40, 50]), [30, 60]),
            (Some([0, 0]), None, [0, 10]),
            (Some([100, 100]), None, [90, 100]),
            (Some([0, 100]), Some([40, 50]), [0, 100]),
            (None, None, [0, 100]),
        ] {
            assert_eq!(humidity_bounds(observed, reference), expected);
        }
    }

    #[test]
    fn chart_anchors_keep_observations_in_time_order() {
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
        let bins = trend::group(rows, 0, 96 * 60).unwrap();
        assert_eq!(
            curve::anchors(&bins)
                .iter()
                .map(|p| (p.at, p.rh))
                .collect::<Vec<_>>(),
            [(0, 50), (60, 52), (120, 46), (180, 47), (240, 51)],
        );
        assert_eq!(curve::anchors(&bins[2..]).len(), 1);
    }
}
