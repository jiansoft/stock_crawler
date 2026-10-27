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

/// 從全部分級取出每檔證券的千張大戶與合計列；兩列缺一的證券不列入。
pub fn summarize(levels: &[HolderLevel]) -> HashMap<String, HolderDistribution> {
    #[derive(Default)]
    struct Partial {
        date: Option<NaiveDate>,
        major: Option<(i64, Decimal)>,
        total: Option<i64>,
    }

    let mut partials: HashMap<&str, Partial> = HashMap::new();
    for level in levels {
        let partial = partials.entry(level.stock_symbol.as_str()).or_default();
        partial.date = Some(level.date);
        match level.level {
            LEVEL_MAJOR => partial.major = Some((level.holders, level.percent)),
            LEVEL_TOTAL => partial.total = Some(level.holders),
            _ => {}
        }
    }

    partials
        .into_iter()
        .filter_map(|(symbol, partial)| {
            let (major_holders, major_percent) = partial.major?;
            Some((
                symbol.to_string(),
                HolderDistribution {
                    date: partial.date?,
                    major_holders,
                    major_percent,
                    total_holders: partial.total?,
                },
            ))
        })
        .collect()
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
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    const FIXTURE: &str = include_str!("testdata/shareholding_distribution.csv");

    /// 2330 2026-10-02：千張大戶 1,485 人、持股 84.77%，股東 3,010,913 人。
    #[test]
    fn summarize_fixture() {
        let distributions = summarize(&parse(FIXTURE));
        assert_eq!(distributions.len(), 2);
        assert_eq!(
            distributions["2330"],
            HolderDistribution {
                date: NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
                major_holders: 1_485,
                major_percent: dec!(84.77),
                total_holders: 3_010_913,
            }
        );
        assert!(distributions.contains_key("2884"));
    }

    /// 每檔 17 列全部保留；第 12 級（400,001～600,000 股）與合計列的數值原樣取出。
    #[test]
    fn parse_keeps_every_level() {
        let levels = parse(FIXTURE);
        assert_eq!(levels.len(), 34);
        let of_2330: Vec<_> = levels.iter().filter(|l| l.stock_symbol == "2330").collect();
        assert_eq!(
            of_2330.iter().map(|l| l.level).collect::<Vec<_>>(),
            (1..=17).collect::<Vec<i16>>()
        );
        assert_eq!(
            *of_2330[11],
            HolderLevel {
                date: NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
                stock_symbol: "2330".to_string(),
                level: 12,
                holders: 582,
                shares: 283_883_740,
                percent: dec!(1.09),
            }
        );
        assert_eq!(of_2330[16].shares, 25_932_370_067);
        assert_eq!((of_2330[15].level, of_2330[15].holders), (16, 2));
    }

    /// 缺千張大戶或合計列的證券不列入摘要；壞列與超出範圍的分級略過。
    #[test]
    fn parse_skips_incomplete_symbols_and_bad_lines() {
        let body = "資料日期,證券代號,持股分級,人數,股數,占集保庫存數比例%\n\
                    20261002,1234  ,15,10,2000000,50.00\n\
                    20261002,5678  ,15,3,3000000,30.00\n\
                    20261002,5678  ,17,900,10000000,100.00\n\
                    20261002,5678  ,18,1,1,0.00\n\
                    20261002,5678  ,0,1,1,0.00\n\
                    20261002,      ,1,1,1,0.00\n\
                    bad,line\n\
                    20261002,9999  ,17,x,1,100.00\n\
                    20261002,9999  ,1,1,y,100.00\n";
        let levels = parse(body);
        assert_eq!(levels.len(), 3);
        let distributions = summarize(&levels);
        assert_eq!(distributions.len(), 1);
        assert_eq!(distributions["5678"].major_percent, dec!(30.00));
        assert_eq!(distributions["5678"].total_holders, 900);
    }

    #[tokio::test]
    #[ignore = "live test：連線真實外部網站，需要時手動執行"]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        let distributions = visit().await.expect("集保股權分散表");
        dbg!(distributions.len(), distributions.get("2330"));
        assert!(distributions.len() > 2_000);
    }
}
