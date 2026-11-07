//! # 集保戶股權分散表
//!
//! 來源：集保開放資料 `https://opendata.tdcc.com.tw/getOD.ashx?id=1-5`，
//! 只有**最新一週**（資料日期為該週最後一個交易日，週六公布），一次回傳全市場約 2.4 MB 的 CSV：
//!
//! ```text
//! 資料日期,證券代號,持股分級,人數,股數,占集保庫存數比例%
//! 20261002,2330  ,15,1485,21984287365,84.77
//! ```
//!
//! 持股分級 1～15 依持股數由小到大（第 15 級為 1,000,001 股以上，即「千張大戶」），
//! 16 是差異數調整，17 是合計。開頭有 BOM，證券代號尾端補空白。

use std::collections::HashMap;

use anyhow::Result;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::core::util::{self, text};

/// 開放資料網址。
const URL: &str = "https://opendata.tdcc.com.tw/getOD.ashx?id=1-5";

/// 400 張以上的第一個持股分級（第 12 級為 400,001～600,000 股；12～15 級合計即 400 張以上）。
const LEVEL_400: i16 = 12;
/// 千張大戶（1,000,001 股以上）的持股分級。
const LEVEL_MAJOR: i16 = 15;
/// 合計的持股分級。
const LEVEL_TOTAL: i16 = 17;

/// 單一證券一週的股權分散摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolderDistribution {
    /// 資料日期。
    pub date: NaiveDate,
    /// 千張大戶人數。
    pub major_holders: i64,
    /// 千張大戶持股佔集保庫存比例（%）。
    pub major_percent: Decimal,
    /// 集保股東總人數。
    pub total_holders: i64,
    /// 400 張以上（第 12～15 級）人數。
    pub holders_400: i64,
    /// 400 張以上（第 12～15 級）股數。
    pub shares_400: i64,
    /// 400 張以上股數佔集保庫存比例（%，由股數重算到小數 4 位）；合計股數為 0 時為 `None`。
    pub percent_400: Option<Decimal>,
}

/// CSV 的一列：單一證券一週、單一持股分級。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolderLevel {
    /// 資料日期。
    pub date: NaiveDate,
    /// 證券代號。
    pub stock_symbol: String,
    /// 持股分級（1～15 依持股數由小到大，16 差異數調整，17 合計）。
    pub level: i16,
    /// 人數。
    pub holders: i64,
    /// 股數。
    pub shares: i64,
    /// 占集保庫存數比例（%）。
    pub percent: Decimal,
}

/// 取得最新一週全市場的股權分散摘要（`代號 → 摘要`）。
///
/// # Errors
///
/// HTTP 請求失敗，或回應裡沒有任何可解析的資料列時回傳錯誤。
pub async fn visit() -> Result<HashMap<String, HolderDistribution>> {
    Ok(summarize(&visit_levels().await?))
}

/// 取得最新一週全市場每檔證券的全部持股分級。
///
/// # Errors
///
/// HTTP 請求失敗，或回應裡沒有任何可解析的資料列時回傳錯誤。
pub async fn visit_levels() -> Result<Vec<HolderLevel>> {
    let body = util::http::get(URL, None).await?;
    let levels = parse(&body);
    anyhow::ensure!(
        !levels.is_empty(),
        "集保股權分散表沒有可解析的資料列: {}",
        text::log_snippet(&body, 200)
    );
    Ok(levels)
}

/// 從全部分級取出每檔證券的千張大戶與合計列，並加總第 12～15 級為 400 張以上；
/// 千張或合計列缺一的證券不列入。
///
/// 400 張以上的比例用股數除以合計股數重算，不加總各級比例（各級比例已四捨五入到小數 2 位）。
pub fn summarize(levels: &[HolderLevel]) -> HashMap<String, HolderDistribution> {
    #[derive(Default)]
    struct Partial {
        date: Option<NaiveDate>,
        major: Option<(i64, Decimal)>,
        total: Option<(i64, i64)>,
        holders_400: i64,
        shares_400: i64,
    }

    let mut partials: HashMap<&str, Partial> = HashMap::new();
    for level in levels {
        let partial = partials.entry(level.stock_symbol.as_str()).or_default();
        partial.date = Some(level.date);
        if (LEVEL_400..=LEVEL_MAJOR).contains(&level.level) {
            partial.holders_400 += level.holders;
            partial.shares_400 += level.shares;
        }
        match level.level {
            LEVEL_MAJOR => partial.major = Some((level.holders, level.percent)),
            LEVEL_TOTAL => partial.total = Some((level.holders, level.shares)),
            _ => {}
        }
    }

    partials
        .into_iter()
        .filter_map(|(symbol, partial)| {
            let (major_holders, major_percent) = partial.major?;
            let (total_holders, total_shares) = partial.total?;
            Some((
                symbol.to_string(),
                HolderDistribution {
                    date: partial.date?,
                    major_holders,
                    major_percent,
                    total_holders,
                    holders_400: partial.holders_400,
                    shares_400: partial.shares_400,
                    percent_400: share_percent(partial.shares_400, total_shares),
                },
            ))
        })
        .collect()
}

/// `shares` 佔 `total` 的百分比，四捨五入到小數 4 位；`total` 不為正數時為 `None`。
fn share_percent(shares: i64, total: i64) -> Option<Decimal> {
    (total > 0)
        .then(|| (Decimal::from(shares) * Decimal::ONE_HUNDRED / Decimal::from(total)).round_dp(4))
}

/// 解析整份 CSV；欄位不足、數字無法解析或分級不在 1～17 的列略過。
fn parse(body: &str) -> Vec<HolderLevel> {
    body.lines().skip(1).filter_map(parse_line).collect()
}

/// 解析一列 `資料日期,代號,分級,人數,股數,比例`。
fn parse_line(line: &str) -> Option<HolderLevel> {
    let mut fields = line.split(',');
    let date = NaiveDate::parse_from_str(
        fields.next()?.trim_start_matches('\u{feff}').trim(),
        "%Y%m%d",
    )
    .ok()?;
    let stock_symbol = fields.next()?.trim().to_string();
    let level: i16 = fields.next()?.trim().parse().ok()?;
    if !(1..=LEVEL_TOTAL).contains(&level) || stock_symbol.is_empty() {
        return None;
    }
    let holders: i64 = fields.next()?.trim().parse().ok()?;
    let shares: i64 = fields.next()?.trim().parse().ok()?;
    let percent = text::parse_decimal(fields.next()?.trim(), None).ok()?;
    Some(HolderLevel {
        date,
        stock_symbol,
        level,
        holders,
        shares,
        percent,
    })
}

#[cfg(test)]
mod tests;
