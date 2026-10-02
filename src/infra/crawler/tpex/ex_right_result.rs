//! # TPEx 上櫃除權除息計算結果表採集器
//!
//! 資料來源為櫃買中心 `www/zh-tw/bulletin/exDailyQ`（「除權除息計算結果表」），已除權息的實際結果。
//! 支援 `startDate`／`endDate` 日期區間（`YYYY/MM/DD`），一次請求可取回一整年。
//!
//! 與證交所不同，這裡直接給「現金股利」與「每仟股無償配股」，可以換算出現金與股票股利。
//! 「除權」但無償配股為 0 的是現金增資，不是股利事件，直接丟棄。

use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{
    core::{
        declare::StockExchangeMarket, util::datetime::parse_taiwan_date, util::http, util::text,
    },
    infra::crawler::{
        share::{ExDividendAnnouncement, parse_ex_dividend_kind},
        tpex,
    },
};

/// `exDailyQ` 的回應主體。
#[derive(Deserialize, Debug)]
pub struct ExDailyQResponse {
    /// 資料表；只有一張。
    pub tables: Option<Vec<ExDailyQTable>>,
}

/// `exDailyQ` 的資料表。
#[derive(Deserialize, Debug)]
pub struct ExDailyQTable {
    /// 資料列：除權息日期、代號、名稱、……、權/息（第 9 欄）、……、現金股利（第 14 欄）、每仟股無償配股（第 15 欄）。
    pub data: Option<Vec<Vec<String>>>,
}

/// 資料列欄位索引。
const COL_DATE: usize = 0;
const COL_CODE: usize = 1;
const COL_NAME: usize = 2;
const COL_KIND: usize = 8;
const COL_CASH: usize = 13;
const COL_STOCK_PER_THOUSAND: usize = 14;
/// 每仟股配股換算成配股率（股/股）。
const THOUSAND: Decimal = Decimal::from_parts(1000, 0, 0, false, 0);

/// 取得 `start`～`end`（含）之間已除權息的上櫃證券。
///
/// # 錯誤
///
/// HTTP 請求或 JSON 反序列化失敗、或回應沒有資料表時回傳錯誤。
pub async fn visit(start: NaiveDate, end: NaiveDate) -> Result<Vec<ExDividendAnnouncement>> {
    let url = format!(
        "https://{}/www/zh-tw/bulletin/exDailyQ?startDate={}&endDate={}&response=json",
        tpex::HOST,
        start.format("%Y/%m/%d"),
        end.format("%Y/%m/%d")
    );
    let response = http::get_json::<ExDailyQResponse>(&url)
        .await
        .with_context(|| format!("Failed to fetch TPEx exDailyQ for {start}~{end}"))?;
    parse_results(&response)
}

/// 將回應轉成除權息事件；單一資料列格式不符只略過該列。
pub fn parse_results(response: &ExDailyQResponse) -> Result<Vec<ExDividendAnnouncement>> {
    let table = response
        .tables
        .as_ref()
        .and_then(|tables| tables.first())
        .context("TPEx exDailyQ 回應沒有資料表")?;
    let rows = table.data.as_deref().unwrap_or_default();

    Ok(rows
        .iter()
        .filter(|row| row.len() > COL_STOCK_PER_THOUSAND)
        .filter_map(|row| {
            let ex_date = parse_taiwan_date(row[COL_DATE].trim())?;
            let cash = number(&row[COL_CASH]);
            let ratio =
                number(&row[COL_STOCK_PER_THOUSAND]).map(|per_thousand| per_thousand / THOUSAND);
            let (kind_cash, kind_stock) = parse_ex_dividend_kind(row[COL_KIND].trim());
            let is_cash = kind_cash && cash.is_some_and(|value| value > Decimal::ZERO);
            let is_stock = kind_stock && ratio.is_some_and(|value| value > Decimal::ZERO);
            if !is_cash && !is_stock {
                return None;
            }
            Some(ExDividendAnnouncement {
                stock_symbol: row[COL_CODE].trim().to_string(),
                name: row[COL_NAME].trim().to_string(),
                ex_date,
                is_cash,
                is_stock,
                cash_dividend: cash.filter(|_| is_cash).map(|value| value.round_dp(4)),
                stock_dividend_ratio: ratio.filter(|_| is_stock),
                market: StockExchangeMarket::OverTheCounter,
            })
        })
        .collect())
}

fn number(raw: &str) -> Option<Decimal> {
    text::parse_decimal(raw.trim(), Some(vec![','])).ok()
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn fixture() -> ExDailyQResponse {
        serde_json::from_str(include_str!("testdata/ex_daily_q_202307_sample.json"))
            .expect("fixture 應為合法 JSON")
    }

    #[test]
    fn parse_results_splits_cash_and_stock_dividends() {
        let results = parse_results(&fixture()).expect("解析成功");
        // 6804 除權但無償配股為 0（現金增資），不算股利事件。
        assert_eq!(results.len(), 3);

        let both = &results[0];
        assert_eq!(both.stock_symbol, "4175");
        assert_eq!(both.ex_date, NaiveDate::from_ymd_opt(2023, 7, 20).unwrap());
        assert!(both.is_cash && both.is_stock);
        assert_eq!(both.cash_dividend, Some(dec!(2.7452)));
        assert_eq!(both.stock_dividend(), Some(dec!(0.9632195232)));
        assert_eq!(both.market, StockExchangeMarket::OverTheCounter);

        let cash_only = &results[1];
        assert_eq!(cash_only.stock_symbol, "1788");
        assert!(cash_only.is_cash && !cash_only.is_stock);
        assert_eq!(cash_only.cash_dividend, Some(dec!(7)));
        assert_eq!(cash_only.stock_dividend_ratio, None);
    }

    #[test]
    fn missing_table_is_an_error() {
        assert!(parse_results(&ExDailyQResponse { tables: None }).is_err());
        assert!(
            parse_results(&ExDailyQResponse {
                tables: Some(vec![])
            })
            .is_err()
        );
        let empty = ExDailyQResponse {
            tables: Some(vec![ExDailyQTable { data: Some(vec![]) }]),
        };
        assert!(parse_results(&empty).expect("空表不是錯誤").is_empty());
    }
}
