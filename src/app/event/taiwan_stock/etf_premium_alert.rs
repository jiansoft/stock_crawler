//! # 持有 ETF 折溢價通知
//!
//! 每個交易日 17:30 的淨值入庫之後，檢查持有的 ETF：
//!
//! - **溢價**：今天的折溢價 ≥ `PREMIUM_THRESHOLD`%，且前一筆還沒達門檻（剛轉為高溢價）。
//! - **折價**：最近 `DISCOUNT_DAYS` 筆都 ≤ `DISCOUNT_THRESHOLD`%，且再前一筆不是（剛好連續滿 5 天）。
//!
//! 都只在「剛發生」那天通知一次，持續溢價或折價不會天天重發。最新一筆不是今天（休市、
//! 淨值尚未公布）時不判斷。

use std::{collections::HashMap, fmt::Write};

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};

use super::holdings;
use crate::{
    core::{alert, util::text},
    domain::etf::{EtfRepository, PremiumPoint},
    infra::database::repository::etf::PgEtfRepository,
};

/// 溢價門檻（%）。
pub const PREMIUM_THRESHOLD: f64 = 2.0;
/// 折價門檻（%）。
pub const DISCOUNT_THRESHOLD: f64 = -1.0;
/// 連續折價天數。
pub const DISCOUNT_DAYS: usize = 5;

/// 一則折溢價通知。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PremiumAlert {
    /// 剛轉為高溢價。
    Premium { symbol: String, rate: f64 },
    /// 剛好連續折價滿 [`DISCOUNT_DAYS`] 天。
    Discount {
        symbol: String,
        rate: f64,
        average: f64,
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
    let premiums = PgEtfRepository::new()
        .recent_premiums(&symbols, DISCOUNT_DAYS as i64 + 1)
        .await?;
    let today = Local::now().date_naive();
    let alerts = detect(&premiums, today);
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
        text::escape_markdown_v2(format!("持有 ETF 折溢價︰{date}"))
    );
    for alert in alerts {
        let line = match alert {
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
        };
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
    }
    Some(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).expect("日期合法")
    }

    /// 由新到舊的折溢價，第一筆是 `latest`。
    fn points(latest: u32, rates: &[f64]) -> Vec<PremiumPoint> {
        rates
            .iter()
            .enumerate()
            .map(|(i, rate)| PremiumPoint {
                date: day(latest - i as u32),
                premium_rate: *rate,
            })
            .collect()
    }

    fn one(symbol: &str, series: Vec<PremiumPoint>) -> HashMap<String, Vec<PremiumPoint>> {
        HashMap::from([(symbol.to_string(), series)])
    }

    #[test]
    fn premium_alerts_only_when_crossing_the_threshold() {
        let crossed = one("00830", points(21, &[2.3, 1.1]));
        assert_eq!(
            detect(&crossed, day(21)),
            vec![PremiumAlert::Premium {
                symbol: "00830".to_string(),
                rate: 2.3
            }]
        );
        let first_ever = one("00830", points(21, &[2.0]));
        assert_eq!(detect(&first_ever, day(21)).len(), 1);
        let continuing = one("00830", points(21, &[2.3, 2.1]));
        assert!(detect(&continuing, day(21)).is_empty());
    }

    #[test]
    fn discount_alerts_when_the_streak_just_reaches_five_days() {
        let reached = one("00679B", points(21, &[-1.2, -1.0, -1.5, -1.1, -1.2, -0.3]));
        match detect(&reached, day(21)).as_slice() {
            [PremiumAlert::Discount { rate, average, .. }] => {
                assert!((rate + 1.2).abs() < 1e-9);
                assert!((average + 1.2).abs() < 1e-9);
            }
            other => panic!("應有一則折價通知：{other:?}"),
        }
        let six_days = one("00679B", points(21, &[-1.2, -1.0, -1.5, -1.1, -1.2, -1.3]));
        assert!(detect(&six_days, day(21)).is_empty());
        let broken = one("00679B", points(21, &[-1.2, -0.9, -1.5, -1.1, -1.2]));
        assert!(detect(&broken, day(21)).is_empty());
        let short = one("00679B", points(21, &[-1.2, -1.3]));
        assert!(detect(&short, day(21)).is_empty());
    }

    #[test]
    fn stale_data_is_ignored() {
        let stale = one("00830", points(20, &[3.0]));
        assert!(detect(&stale, day(21)).is_empty());
        assert!(detect(&one("00830", Vec::new()), day(21)).is_empty());
    }

    #[test]
    fn message_lists_alerts_in_order() {
        let alerts = vec![
            PremiumAlert::Premium {
                symbol: "00830".to_string(),
                rate: 2.31,
            },
            PremiumAlert::Discount {
                symbol: "00679B".to_string(),
                rate: -1.2,
                average: -1.25,
            },
        ];
        let message = build_message(day(21), &alerts, |s| format!("名稱{s}")).expect("有訊息");
        assert!(message.contains("持有 ETF 折溢價︰2026\\-10\\-21"));
        assert!(message.contains("00830 名稱00830 溢價 \\+2\\.31%"));
        assert!(
            message.contains("00679B 名稱00679B 連續 5 日折價，今日 \\-1\\.20%、平均 \\-1\\.25%")
        );
        assert!(build_message(day(21), &[], |_| String::new()).is_none());
    }
}
