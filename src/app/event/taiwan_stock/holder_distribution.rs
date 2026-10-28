//! # 持股大戶持股比例週報
//!
//! 每週六集保公布上週五的戶股權分散表後，先把全市場寫進資料庫，再列出每檔持股的
//! 千張與 400 張以上大戶持股比例、人數與股東總數，和上週比較，看籌碼是往大戶集中還是往散戶分散。
//!
//! ## 流程（每週六 10:40 排程）
//!
//! 1. 集保股權分散入庫（[`chip::execute_holder_distributions`]）；失敗就不送週報，錯誤交給排程告警。
//! 2. 取目前持股的普通股代號（[`holdings::common_stock_holdings`]）。
//! 3. 從 `holder_distribution` 讀最近 5 個資料週：最新一週是本週，每檔在本週之前最近的一筆是上週，
//!    全表往前第 4 個資料週是 4 週前；缺哪一週就不顯示那一項變化。
//! 4. 依千張大戶比例的週變化由增到減排列，一則訊息列出全部持股；沒有上週資料時只列現況。
//! 5. 以資料日期做去重（Redis `tdcc:holder_report:{日期}`）：集保還沒更新、入庫後最新一週仍是
//!    已通知過的那週時不再送。

use std::{collections::HashMap, fmt::Write};

use anyhow::{Context, Result};
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;

use super::{add_thousand_separators, format_decimal_with_commas, holdings};
use crate::{
    app::backfill::chip,
    core::{alert, util::text},
    domain::chip::{HolderDistributionRecord, HolderHistoryRepository, RecentHolderWeeks},
    infra::{database::repository::chip::PgChipRepository, nosql::redis::CLIENT},
};

/// 讀幾個資料週：本週加往前 4 週。
const WEEKS: i64 = 5;

/// 4 週變化的基準是往前第幾個資料週。
const FOUR_WEEKS: usize = 4;

/// 去重鍵的保存時間（30 天）。
const REPORTED_TTL_SECONDS: usize = 60 * 60 * 24 * 30;

/// 單一持股的週報列。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    stock_symbol: String,
    current: HolderDistributionRecord,
    previous: Option<HolderDistributionRecord>,
    four_weeks_ago: Option<HolderDistributionRecord>,
}

impl Row {
    /// 千張大戶比例的週變化（百分點）；沒有上週資料時為 `None`。
    fn major_change(&self) -> Option<Decimal> {
        self.previous
            .as_ref()
            .map(|previous| self.current.major_percent - previous.major_percent)
    }

    /// 千張大戶比例的 4 週變化（百分點）；4 週前沒有資料時為 `None`。
    fn major_change_four_weeks(&self) -> Option<Decimal> {
        self.four_weeks_ago
            .as_ref()
            .map(|before| self.current.major_percent - before.major_percent)
    }

    /// 400 張以上比例的週變化（百分點）；本週或上週沒有 400 張資料時為 `None`。
    fn change_400(&self) -> Option<Decimal> {
        let current = self.current.percent_400?;
        let previous = self.previous.as_ref()?.percent_400?;
        Some(current - previous)
    }

    /// 400 張以上人數的週增減；本週或上週沒有 400 張資料時為 `None`。
    fn holders_400_change(&self) -> Option<i64> {
        let current = self.current.holders_400?;
        let previous = self.previous.as_ref()?.holders_400?;
        Some(current - previous)
    }
}

/// 排程入口：集保股權分散入庫，成功後彙整持股的大戶比例週變化並通知。
///
/// # Errors
///
/// 入庫、查詢持股或讀取週資料失敗時回傳錯誤（交給排程告警）；去重鍵讀寫失敗只記錄，照常送出。
pub async fn execute() -> Result<()> {
    chip::execute_holder_distributions()
        .await
        .context("集保股權分散入庫失敗，本週持股大戶週報不送")?;
    let symbols: Vec<String> = holdings::common_stock_holdings()
        .await
        .context("fetch active holdings for holder distribution report failed")?
        .into_iter()
        .collect();
    let Some((date, rows)) = collect_rows(
        &PgChipRepository::new(),
        &symbols,
        Local::now().date_naive(),
    )
    .await?
    else {
        tracing::info!("集保股權分散沒有任何持股的本週資料，略過大戶週報");
        return Ok(());
    };
    if !mark_reported(date).await {
        tracing::info!("集保股權分散最新仍是 {date}，這週已通知過");
        return Ok(());
    }
    alert::send_message(&build_message(date, &rows, holdings::stock_name)).await;
    tracing::info!(
        "持股大戶週報結束: date={date}, holdings={}, reported={}",
        symbols.len(),
        rows.len()
    );
    Ok(())
}

/// 讀最近幾週的週摘要並組成排好序的週報列；沒有資料週或沒有任何持股的本週資料時回傳 `None`。
async fn collect_rows(
    repository: &impl HolderHistoryRepository,
    symbols: &[String],
    as_of: NaiveDate,
) -> Result<Option<(NaiveDate, Vec<Row>)>> {
    let weeks = repository
        .recent_holder_weeks(symbols, as_of, WEEKS)
        .await
        .context("fetch recent holder distribution weeks failed")?;
    let Some(&date) = weeks.dates.first() else {
        return Ok(None);
    };
    let mut rows = build_rows(symbols, &weeks);
    if rows.is_empty() {
        return Ok(None);
    }
    sort_rows(&mut rows);
    Ok(Some((date, rows)))
}

/// 把週資料依持股整理成週報列；本週沒有資料的持股（停牌、ETF 以外的特殊代號）不列。
fn build_rows(symbols: &[String], weeks: &RecentHolderWeeks) -> Vec<Row> {
    let Some(&latest) = weeks.dates.first() else {
        return Vec::new();
    };
    let four_weeks_date = weeks.dates.get(FOUR_WEEKS).copied();
    let mut by_symbol: HashMap<&str, Vec<&HolderDistributionRecord>> = HashMap::new();
    for record in &weeks.records {
        by_symbol
            .entry(record.stock_symbol.as_str())
            .or_default()
            .push(record);
    }
    symbols
        .iter()
        .filter_map(|symbol| {
            let records = by_symbol.get(symbol.as_str())?;
            let current = records.iter().find(|record| record.date == latest)?;
            let previous = records
                .iter()
                .filter(|record| record.date < latest)
                .max_by_key(|record| record.date);
            let four_weeks_ago =
                four_weeks_date.and_then(|date| records.iter().find(|record| record.date == date));
            Some(Row {
                stock_symbol: symbol.clone(),
                current: (*current).clone(),
                previous: previous.map(|record| (*record).clone()),
                four_weeks_ago: four_weeks_ago.map(|record| (*record).clone()),
            })
        })
        .collect()
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

/// 組出週報（Telegram MarkdownV2）。每檔兩行：千張一行，400 張與股東人數一行。
///
/// `stock_name` 由呼叫端提供（正式流程讀股票主檔快取），測試時可換成固定對照。
fn build_message(date: NaiveDate, rows: &[Row], stock_name: impl Fn(&str) -> String) -> String {
    let mut message = String::with_capacity(256 + rows.len() * 192);
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2(format!("持股大戶週報︰{date}（集保股權分散表）"))
    );
    let _ = writeln!(
        &mut message,
        "{}",
        text::escape_markdown_v2(
            "千張、400 張：持股比例（週變化，4 週變化，百分點）／人數（週增減）；股東人數（週變化）"
        )
    );
    for row in rows {
        let current = &row.current;
        let previous = row.previous.as_ref();
        let mut major_changes = Vec::new();
        if let Some(change) = row.major_change() {
            major_changes.push(signed_points(change));
        }
        if let Some(change) = row.major_change_four_weeks() {
            major_changes.push(format!("4 週 {}", signed_points(change)));
        }
        let major = format!(
            "{} {} 千張 {}%{}／{} 人{}",
            row.stock_symbol,
            stock_name(&row.stock_symbol),
            format_decimal_with_commas(current.major_percent),
            parenthesized(&major_changes.join("，")),
            add_thousand_separators(&current.major_holders.to_string()),
            previous
                .map(|previous| parenthesized(&signed_count(
                    current.major_holders - previous.major_holders
                )))
                .unwrap_or_default()
        );
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(major));

        let mut detail = String::from("　");
        if let (Some(percent), Some(holders)) = (current.percent_400, current.holders_400) {
            let _ = write!(
                detail,
                "400 張 {}%{}／{} 人{}／",
                format_decimal_with_commas(percent),
                row.change_400()
                    .map(|change| parenthesized(&signed_points(change)))
                    .unwrap_or_default(),
                add_thousand_separators(&holders.to_string()),
                row.holders_400_change()
                    .map(|change| parenthesized(&signed_count(change)))
                    .unwrap_or_default()
            );
        }
        let _ = write!(
            detail,
            "股東 {} 人{}",
            add_thousand_separators(&current.total_holders.to_string()),
            previous
                .map(|previous| holders_change(previous.total_holders, current.total_holders))
                .unwrap_or_default()
        );
        let _ = writeln!(&mut message, "{}", text::escape_markdown_v2(detail));
    }
    message
}

/// 有內容時加上全形括號，空字串維持空字串。
fn parenthesized(content: &str) -> String {
    if content.is_empty() {
        String::new()
    } else {
        format!("（{content}）")
    }
}

/// 帶正負號的百分點，例如 `+0.32`、`-1.5`、`0`。
fn signed_points(change: Decimal) -> String {
    let sign = if change > Decimal::ZERO { "+" } else { "" };
    format!("{sign}{}", format_decimal_with_commas(change))
}

/// 帶正負號、千分位的人數增減，例如 `+1,200`、`-3`、`0`。
fn signed_count(change: i64) -> String {
    let sign = if change > 0 { "+" } else { "" };
    format!("{sign}{}", add_thousand_separators(&change.to_string()))
}

/// 股東人數週變化比例，例如「（-1.2%）」；上週為 0 或沒有變化時不顯示。
fn holders_change(previous: i64, current: i64) -> String {
    if previous == 0 || previous == current {
        return String::new();
    }
    let percent = (current - previous) as f64 * 100.0 / previous as f64;
    format!("（{percent:+.1}%）")
}

/// 這週已通知的去重鍵。
fn reported_key(date: NaiveDate) -> String {
    format!("tdcc:holder_report:{date}")
}

/// 記下這個資料週已通知；回傳 `false` 代表先前已通知過。Redis 失敗時寧可重送，記錄後回傳 `true`。
async fn mark_reported(date: NaiveDate) -> bool {
    match CLIENT
        .set_if_absent(reported_key(date), "1", REPORTED_TTL_SECONDS)
        .await
    {
        Ok(first) => first,
        Err(why) => {
            tracing::warn!("寫入集保週報去重鍵失敗，照常送出: {why:?}");
            true
        }
    }
}

#[cfg(test)]
mod tests;
