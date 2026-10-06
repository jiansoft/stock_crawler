//! # 持股大戶持股比例週報
//!
//! 每週六抓集保戶股權分散表，列出每檔持股的千張大戶持股比例、大戶人數與股東總數，
//! 並和上週比較，看籌碼是往大戶集中還是往散戶分散。
//!
//! ## 流程（每週六 10:30 排程）
//!
//! 1. 取目前持股的普通股代號（[`holdings::common_stock_holdings`]）。
//! 2. 抓集保開放資料（只有最新一週、全市場一次回傳）。
//! 3. 與 Redis 保存的上週快照（`tdcc:snapshot:{代號}`）比較；資料日期相同代表這週已通知過，整份略過。
//! 4. 依千張大戶比例的週變化由增到減排列，一則訊息列出全部持股；第一次沒有上週資料時只列現況。
//! 5. 送出後寫回這週的快照。

use std::fmt::Write;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::{add_thousand_separators, format_decimal_with_commas, holdings};
use crate::{
    core::{alert, util::text},
    infra::{
        crawler::tdcc::shareholding::{self, HolderDistribution},
        nosql::redis::{CLIENT, RedisError},
    },
};

/// 上週快照的保存時間（60 天），漏掉一兩週仍比得到前一份。
const SNAPSHOT_TTL_SECONDS: usize = 60 * 60 * 24 * 60;

/// 存進 Redis 的每週快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Snapshot {
    date: NaiveDate,
    major_holders: i64,
    major_percent: Decimal,
    total_holders: i64,
}

impl From<&HolderDistribution> for Snapshot {
    fn from(distribution: &HolderDistribution) -> Self {
        Self {
            date: distribution.date,
            major_holders: distribution.major_holders,
            major_percent: distribution.major_percent,
            total_holders: distribution.total_holders,
        }
    }
}

/// 單一持股的週報列。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    stock_symbol: String,
    current: Snapshot,
    previous: Option<Snapshot>,
}

impl Row {
    /// 千張大戶比例的週變化（百分點）；沒有上週資料時為 `None`。
    fn major_change(&self) -> Option<Decimal> {
        self.previous
            .as_ref()
            .map(|previous| self.current.major_percent - previous.major_percent)
    }
}

/// 排程入口：彙整持股的千張大戶比例週變化並通知。
///
/// # Errors
///
/// 查詢持股或抓取集保資料失敗時回傳錯誤（交給排程告警）；單檔讀寫快照失敗只記錄。
pub async fn execute() -> Result<()> {
    let symbols = holdings::common_stock_holdings()
        .await
        .context("fetch active holdings for holder distribution report failed")?;
    let distributions = shareholding::visit()
        .await
        .context("fetch TDCC shareholding distribution failed")?;

    let mut rows = Vec::new();
    for symbol in &symbols {
        let Some(distribution) = distributions.get(symbol) else {
            continue;
        };
        rows.push(Row {
            stock_symbol: symbol.clone(),
            current: Snapshot::from(distribution),
            previous: load_snapshot(symbol).await,
        });
    }
    if rows.is_empty() {
        tracing::info!("集保股權分散表沒有任何持股的資料，略過大戶週報");
        return Ok(());
    }
    if rows.iter().all(|row| {
        row.previous
            .as_ref()
            .is_some_and(|previous| previous.date == row.current.date)
    }) {
        tracing::info!("集保股權分散表仍是 {}，這週已通知過", rows[0].current.date);
        return Ok(());
    }

    sort_rows(&mut rows);
    alert::send_message(&build_message(&rows, holdings::stock_name)).await;
    for row in &rows {
        save_snapshot(&row.stock_symbol, &row.current).await;
    }
    tracing::info!(
        "持股大戶週報結束: holdings={}, reported={}",
        symbols.len(),
        rows.len()
    );
    Ok(())
}

/// 依千張大戶比例的週變化由增到減排列；沒有上週資料的排在最後，依代號排序。
fn sort_rows(rows: &mut [Row]) {
    rows.sort_by(|a, b| match (a.major_change(), b.major_change()) {
        (Some(a_change), Some(b_change)) => b_change.cmp(&a_change),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.stock_symbol.cmp(&b.stock_symbol),
    });
}

/// 組出週報（Telegram MarkdownV2）。
///
/// `stock_name` 由呼叫端提供（正式流程讀股票主檔快取），測試時可換成固定對照。
fn build_message(rows: &[Row], stock_name: impl Fn(&str) -> String) -> String {
    let date = rows
        .iter()
        .map(|row| row.current.date)
        .max()
        .unwrap_or_default();
    let mut message = String::with_capacity(256 + rows.len() * 128);
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2(format!("持股千張大戶週報︰{date}（集保股權分散表）"))
    );
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2("大戶比例（週變化）／大戶人數／股東人數（週變化）")
    );
    for row in rows {
        let current = &row.current;
        let previous = row.previous.as_ref();
        let line = format!(
            "{} {} {}%{}／{} 人／{} 人{}",
            row.stock_symbol,
            stock_name(&row.stock_symbol),
            format_decimal_with_commas(current.major_percent),
            row.major_change()
                .map(|change| format!("（{}）", signed_points(change)))
                .unwrap_or_default(),
            add_thousand_separators(&current.major_holders.to_string()),
            add_thousand_separators(&current.total_holders.to_string()),
            previous
                .map(|previous| holders_change(previous.total_holders, current.total_holders))
                .unwrap_or_default()
        );
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(line));
    }
    message
}

/// 帶正負號的百分點，例如 `+0.32`、`-1.5`、`0`。
fn signed_points(change: Decimal) -> String {
    let sign = if change > Decimal::ZERO { "+" } else { "" };
    format!("{sign}{}", format_decimal_with_commas(change))
}

/// 股東人數週變化比例，例如「（-1.2%）」；上週為 0 或沒有變化時不顯示。
fn holders_change(previous: i64, current: i64) -> String {
    if previous == 0 || previous == current {
        return String::new();
    }
    let percent = (current - previous) as f64 * 100.0 / previous as f64;
    format!("（{percent:+.1}%）")
}

/// 上週快照的 Redis key。
fn snapshot_key(stock_symbol: &str) -> String {
    format!("tdcc:snapshot:{stock_symbol}")
}

/// 讀取上週快照；不存在或讀取失敗時回傳 `None`。
async fn load_snapshot(stock_symbol: &str) -> Option<Snapshot> {
    match CLIENT.get_bytes(&snapshot_key(stock_symbol)).await {
        Ok(bytes) => serde_json::from_slice(&bytes).ok(),
        Err(RedisError::NotFound) => None,
        Err(why) => {
            tracing::warn!("讀取 {stock_symbol} 集保快照失敗: {why}");
            None
        }
    }
}

/// 寫回這週快照；失敗只記錄。
async fn save_snapshot(stock_symbol: &str, snapshot: &Snapshot) {
    let Ok(json) = serde_json::to_string(snapshot) else {
        return;
    };
    if let Err(why) = CLIENT
        .set(&snapshot_key(stock_symbol), json, SNAPSHOT_TTL_SECONDS)
        .await
    {
        tracing::warn!("寫入 {stock_symbol} 集保快照失敗: {why:?}");
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn snapshot(
        day: u32,
        major_holders: i64,
        major_percent: Decimal,
        total_holders: i64,
    ) -> Snapshot {
        Snapshot {
            date: NaiveDate::from_ymd_opt(2026, 10, day).unwrap(),
            major_holders,
            major_percent,
            total_holders,
        }
    }

    fn row(symbol: &str, current: Snapshot, previous: Option<Snapshot>) -> Row {
        Row {
            stock_symbol: symbol.to_string(),
            current,
            previous,
        }
    }

    /// 大戶比例增加最多的在前、減少最多的在後，沒有上週資料的排最後。
    #[test]
    fn sort_rows_by_major_change() {
        let mut rows = vec![
            row(
                "2884",
                snapshot(2, 700, dec!(60.0), 800_000),
                Some(snapshot(1, 700, dec!(61.0), 790_000)),
            ),
            row("6505", snapshot(2, 10, dec!(90.0), 300_000), None),
            row(
                "2330",
                snapshot(2, 1_485, dec!(84.77), 3_010_913),
                Some(snapshot(1, 1_480, dec!(84.45), 3_047_000)),
            ),
            row(
                "1101",
                snapshot(2, 300, dec!(50.0), 400_000),
                Some(snapshot(1, 300, dec!(50.0), 400_000)),
            ),
        ];
        sort_rows(&mut rows);
        let order: Vec<&str> = rows.iter().map(|row| row.stock_symbol.as_str()).collect();
        assert_eq!(order, vec!["2330", "1101", "2884", "6505"]);
    }

    #[test]
    fn formatting_helpers() {
        assert_eq!(signed_points(dec!(0.32)), "+0.32");
        assert_eq!(signed_points(dec!(-1.5)), "-1.5");
        assert_eq!(signed_points(Decimal::ZERO), "0");
        assert_eq!(holders_change(3_047_000, 3_010_913), "（-1.2%）");
        assert_eq!(holders_change(0, 10), "");
        assert_eq!(holders_change(10, 10), "");
        assert_eq!(snapshot_key("2330"), "tdcc:snapshot:2330");
    }

    /// 週報列出比例、週變化、大戶人數與股東人數；第一次沒有上週資料時只列現況。
    #[test]
    fn build_message_lists_every_holding() {
        let rows = vec![
            row(
                "2330",
                snapshot(2, 1_485, dec!(84.77), 3_010_913),
                Some(snapshot(1, 1_480, dec!(84.45), 3_047_000)),
            ),
            row("6505", snapshot(2, 10, dec!(90.0), 300_000), None),
        ];
        let message = build_message(&rows, |symbol| match symbol {
            "2330" => "台積電".to_string(),
            _ => "台塑化".to_string(),
        });

        assert!(
            message.starts_with("持股千張大戶週報︰2026\\-10\\-02（集保股權分散表）\n"),
            "{message}"
        );
        assert!(
            message.contains(
                "\n2330 台積電 84\\.77%（\\+0\\.32）／1,485 人／3,010,913 人（\\-1\\.2%）\n"
            ),
            "{message}"
        );
        assert!(
            message.contains("\n6505 台塑化 90%／10 人／300,000 人\n"),
            "{message}"
        );
    }

    /// 快照寫入 Redis 後讀得回來；測試用假代號，結束時刪除。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn snapshot_round_trips_through_redis() {
        dotenvy::dotenv().ok();
        if CLIENT.ping().await.is_err() {
            println!("跳過 snapshot_round_trips_through_redis：無 Redis 連線");
            return;
        }
        let symbol = "79974";
        let _ = CLIENT.delete(&snapshot_key(symbol)).await;
        assert_eq!(load_snapshot(symbol).await, None);

        let saved = snapshot(2, 1_485, dec!(84.77), 3_010_913);
        save_snapshot(symbol, &saved).await;
        assert_eq!(load_snapshot(symbol).await, Some(saved));

        CLIENT
            .delete(&snapshot_key(symbol))
            .await
            .expect("清除測試紀錄");
    }
}
