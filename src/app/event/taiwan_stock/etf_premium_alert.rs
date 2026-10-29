//! # 持有 ETF 折溢價與大額申贖通知
//!
//! 每個交易日 17:30 的淨值入庫之後，檢查持有的 ETF：
//!
//! - **溢價**：今天的折溢價 ≥ `PREMIUM_THRESHOLD`%，且前一筆還沒達門檻（剛轉為高溢價）。
//! - **折價**：最近 `DISCOUNT_DAYS` 筆都 ≤ `DISCOUNT_THRESHOLD`%，且再前一筆不是（剛好連續滿 5 天）。
//! - **大額申贖**：今天的受益單位增減達前一營業日單位數的 ±`FLOW_THRESHOLD`%（單日事件，每次都通知）。
//!
//! 折溢價都只在「剛發生」那天通知一次，持續溢價或折價不會天天重發。最新一筆不是今天（休市、
//! 淨值尚未公布）時不判斷。

use std::{collections::HashMap, fmt::Write};

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};

use super::holdings;
use crate::{
    core::{alert, util::text},
    domain::etf::{EtfRepository, PremiumPoint, UnitsChangePoint},
    infra::database::repository::etf::PgEtfRepository,
};

/// 溢價門檻（%）。
pub const PREMIUM_THRESHOLD: f64 = 2.0;
/// 折價門檻（%）。
pub const DISCOUNT_THRESHOLD: f64 = -1.0;
/// 連續折價天數。
pub const DISCOUNT_DAYS: usize = 5;
/// 大額申贖門檻：單日受益單位增減佔前一營業日單位數（%）。
pub const FLOW_THRESHOLD: f64 = 3.0;
/// 連續三個交易日同向折溢價異常的絕對門檻（%）。
pub const PERSISTENT_THRESHOLD: f64 = 1.5;

/// 一則折溢價通知。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PremiumAlert {
    /// 連續三個交易日同向超過門檻，僅在剛滿三日時通知。
    Persistent {
        symbol: String,
        rate: f64,
        average: f64,
    },
    /// 剛轉為高溢價。
    Premium { symbol: String, rate: f64 },
    /// 剛好連續折價滿 [`DISCOUNT_DAYS`] 天。
    Discount {
        symbol: String,
        rate: f64,
        average: f64,
    },
    /// 單日淨申購（正）或淨贖回（負）達 [`FLOW_THRESHOLD`]%。
    Flow {
        symbol: String,
        /// 單位增減佔前一營業日單位數（%）。
        ratio: f64,
        /// 單位增減 × 淨值（元）；沒有淨值時為 `None`。
        amount: Option<f64>,
    },
}

/// 排程入口。
///
/// # Errors
///
/// 查詢持股或折溢價失敗時回傳錯誤。
pub async fn execute() -> Result<()> {
    let symbols: Vec<String> = holdings::active_holding_symbols()
        .await
        .context("fetch active holdings for ETF premium notification failed")?
        .into_iter()
        .filter(|symbol| symbol.starts_with("00"))
        .collect();
    if symbols.is_empty() {
        return Ok(());
    }
    let repo = PgEtfRepository::new();
    let premiums = repo
        .recent_premiums(&symbols, DISCOUNT_DAYS as i64 + 1)
        .await?;
    let units = repo.latest_units(&symbols).await?;
    let today = Local::now().date_naive();
    let mut alerts = detect(&premiums, today);
    // 交易日曆只供連續折溢價判斷使用，查詢失敗時其餘通知照常發送。
    let calendar = repo
        .recent_trading_dates(today, 4)
        .await
        .unwrap_or_else(|why| {
            tracing::warn!("查詢交易日曆失敗，略過連續折溢價判斷: {why:#}");
            Vec::new()
        });
    merge_persistent(&mut alerts, detect_persistent(&premiums, today, &calendar));
    alerts.extend(detect_flows(&units, today));
    tracing::info!(
        "持有 ETF 折溢價通知: etfs={}, alerts={}",
        symbols.len(),
        alerts.len()
    );
    if let Some(message) = build_message(today, &alerts, holdings::stock_name) {
        alert::send_message(&message).await;
    }
    Ok(())
}

/// 依代號排序找出今天剛發生的溢價或連續折價；`premiums` 每檔由新到舊。
pub(crate) fn detect(
    premiums: &HashMap<String, Vec<PremiumPoint>>,
    today: NaiveDate,
) -> Vec<PremiumAlert> {
    let mut symbols: Vec<&String> = premiums.keys().collect();
    symbols.sort();
    symbols
        .into_iter()
        .filter_map(|symbol| {
            let points = &premiums[symbol];
            let latest = points.first().filter(|p| p.date == today)?;
            let previous = points.get(1).map(|p| p.premium_rate);
            if latest.premium_rate >= PREMIUM_THRESHOLD
                && previous.is_none_or(|rate| rate < PREMIUM_THRESHOLD)
            {
                return Some(PremiumAlert::Premium {
                    symbol: symbol.clone(),
                    rate: latest.premium_rate,
                });
            }
            let streak = points.get(..DISCOUNT_DAYS)?;
            let all_discount = streak.iter().all(|p| p.premium_rate <= DISCOUNT_THRESHOLD);
            let before = points.get(DISCOUNT_DAYS).map(|p| p.premium_rate);
            (all_discount && before.is_none_or(|rate| rate > DISCOUNT_THRESHOLD)).then(|| {
                PremiumAlert::Discount {
                    symbol: symbol.clone(),
                    rate: latest.premium_rate,
                    average: streak.iter().map(|p| p.premium_rate).sum::<f64>()
                        / DISCOUNT_DAYS as f64,
                }
            })
        })
        .collect()
}

/// 依代號排序找出今天單日申贖達門檻的 ETF。
pub(crate) fn detect_flows(
    units: &HashMap<String, UnitsChangePoint>,
    today: NaiveDate,
) -> Vec<PremiumAlert> {
    let mut symbols: Vec<&String> = units.keys().collect();
    symbols.sort();
    symbols
        .into_iter()
        .filter_map(|symbol| {
            let point = units.get(symbol).filter(|p| p.date == today)?;
            let before = point.outstanding_units - point.units_change;
            if before <= 0 {
                return None;
            }
            let ratio = point.units_change as f64 / before as f64 * 100.0;
            (ratio.abs() >= FLOW_THRESHOLD).then(|| PremiumAlert::Flow {
                symbol: symbol.clone(),
                ratio,
                amount: point.nav.map(|nav| point.units_change as f64 * nav),
            })
        })
        .collect()
}

/// 以交易日曆驗證連續性；缺任一天資料、方向反轉或舊資料不發送。
fn detect_persistent(
    premiums: &HashMap<String, Vec<PremiumPoint>>,
    today: NaiveDate,
    calendar: &[NaiveDate],
) -> Vec<PremiumAlert> {
    if calendar.len() < 3 || calendar[0] != today {
        return Vec::new();
    }
    let mut alerts = Vec::new();
    for (symbol, points) in premiums {
        let Some(streak) = points.get(..3) else {
            continue;
        };
        let direction = streak[0].premium_rate.signum();
        let qualifies = |p: &PremiumPoint| {
            p.premium_rate.is_finite() && p.premium_rate * direction > PERSISTENT_THRESHOLD
        };
        if !streak
            .iter()
            .zip(calendar)
            .all(|(p, date)| p.date == *date && qualifies(p))
        {
            continue;
        }
        if points
            .get(3)
            .zip(calendar.get(3))
            .is_some_and(|(p, date)| p.date == *date && qualifies(p))
        {
            continue;
        }
        alerts.push(PremiumAlert::Persistent {
            symbol: symbol.clone(),
            rate: streak[0].premium_rate,
            average: streak.iter().map(|p| p.premium_rate).sum::<f64>() / 3.0,
        });
    }
    alerts.sort_by(|a, b| match (a, b) {
        (
            PremiumAlert::Persistent { symbol: a, .. },
            PremiumAlert::Persistent { symbol: b, .. },
        ) => a.cmp(b),
        _ => std::cmp::Ordering::Equal,
    });
    alerts
}

/// 併入連續折溢價通知；同一檔今天已有溢價或折價通知時略過，避免同一檔出現兩行重疊的訊息。
fn merge_persistent(alerts: &mut Vec<PremiumAlert>, persistent: Vec<PremiumAlert>) {
    let alerted: Vec<String> = alerts
        .iter()
        .filter_map(|alert| match alert {
            PremiumAlert::Premium { symbol, .. } | PremiumAlert::Discount { symbol, .. } => {
                Some(symbol.clone())
            }
            _ => None,
        })
        .collect();
    alerts.extend(persistent.into_iter().filter(|alert| {
        !matches!(alert, PremiumAlert::Persistent { symbol, .. } if alerted.contains(symbol))
    }));
}

/// 組出通知訊息（Telegram MarkdownV2）；沒有通知時回 `None`。
pub(crate) fn build_message(
    date: NaiveDate,
    alerts: &[PremiumAlert],
    stock_name: impl Fn(&str) -> String,
) -> Option<String> {
    if alerts.is_empty() {
        return None;
    }
    let mut message = String::with_capacity(96 + alerts.len() * 96);
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2(format!("持有 ETF 折溢價與申贖︰{date}"))
    );
    for alert in alerts {
        let line = match alert {
            PremiumAlert::Persistent {
                symbol,
                rate,
                average,
            } => format!(
                "{symbol} {} 連續 3 個交易日{}超過 {PERSISTENT_THRESHOLD}%，今日 {rate:+.2}%、平均 {average:+.2}%",
                stock_name(symbol),
                if *rate > 0.0 { "溢價" } else { "折價" }
            ),
            PremiumAlert::Premium { symbol, rate } => format!(
                "{symbol} {} 溢價 {rate:+.2}%（≥ {PREMIUM_THRESHOLD}%），市價明顯高於淨值，加碼前留意",
                stock_name(symbol)
            ),
            PremiumAlert::Discount {
                symbol,
                rate,
                average,
            } => format!(
                "{symbol} {} 連續 {DISCOUNT_DAYS} 日折價，今日 {rate:+.2}%、平均 {average:+.2}%",
                stock_name(symbol)
            ),
            PremiumAlert::Flow {
                symbol,
                ratio,
                amount,
            } => format!(
                "{symbol} {} 單日{} {ratio:+.2}%（受益單位{}，≥ {FLOW_THRESHOLD}%）",
                stock_name(symbol),
                if *ratio > 0.0 {
                    "淨申購"
                } else {
                    "淨贖回"
                },
                amount.map_or_else(String::new, |a| format!("約 {:+.1} 億", a / 1e8))
            ),
        };
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
    }
    Some(message)
}

#[cfg(test)]
mod tests;
