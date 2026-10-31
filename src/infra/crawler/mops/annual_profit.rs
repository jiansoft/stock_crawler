//! # MOPS 年度財報採集
//!
//! 此模組透過公開資訊觀測站「財務比較 E 點通」抓取年度財報資料。
//!
//! 目前採用兩種官方端點：
//! - `/compare/data`：取得年度趨勢資料（Q4 累計），用於 `EPS`、`Revenue`、`CommonStock`
//! - `/compare/report`：取得指定年度綜合損益表 HTML，用於擷取 `稅前淨利（淨損）`
//!
//! 目前回填策略：
//! - `earnings_per_share`：直接使用 `EPS`
//! - `sales_per_share`：以 `Revenue / CommonStock * 10` 推算
//! - `profit_before_tax`：先抓稅前與稅後損益總額，再以 `稅後淨利 ÷ 稅後 EPS`
//!   反推加權平均股數，換算每股稅前淨利；
//!   若特定公司在該端點回空表，則以 `0` 作為保守 fallback
//!
//! ## 公式說明
//!
//! MOPS `compare/data` 與 `compare/report` 取得的多數金額單位為「仟元」，
//! 因此在換算成每股數值時，需要注意單位是否可直接相消。
//!
//! ### 每股營收
//! - `Revenue`：營業收入（仟元）
//! - `CommonStock`：股本（仟元）
//! - 以台股普通股面額 `10` 元估算股數
//!
//! 公式：
//! `每股營收 = Revenue / (CommonStock / 10) = Revenue * 10 / CommonStock`
//!
//! ### 每股稅前淨利
//! - `profit_before_tax_total`：稅前淨利總額（仟元）
//! - `profit_for_eps_total`：與稅後 EPS 對應的淨利總額（仟元）
//! - `earnings_per_share`：稅後每股盈餘（元）
//!
//! 先以 `profit_for_eps_total / earnings_per_share` 反推「加權平均股數（仟股）」，
//! 再以 `profit_before_tax_total / weighted_average_shares`
//! 得到每股稅前淨利（元）。

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::infra::crawler::{
    mops::HOST,
    share::{self, AnnualProfitFetcher},
};

mod parse;

use self::parse::{
    IncomeStatementMetrics, calc_profit_before_tax_per_share, calc_sales_per_share,
    calc_weighted_average_shares, parse_compare_series, parse_income_statement_metrics_from_report,
};

/// MOPS 年度財報抓取器標記型別。
pub struct Mops {}

/// 取得指定股票的年度財報資料。
///
/// 此函式會整合 MOPS 的趨勢資料與綜合損益表資料，
/// 回填 `AnnualProfit` 需要的三個欄位：
/// - `sales_per_share`
/// - `earnings_per_share`
/// - `profit_before_tax`
///
/// `profit_before_tax` 會先反推加權平均股數，再換算成每股值，
/// 避免直接使用股本推估造成偏差。
pub async fn visit(stock_symbol: &str) -> Result<Vec<share::AnnualProfit>> {
    let eps_by_year = fetch_compare_series(stock_symbol, "EPS", "元").await?;
    if eps_by_year.is_empty() {
        return Err(anyhow!(
            "Failed to fetch MOPS annual EPS because response is empty"
        ));
    }

    let revenue_by_year = fetch_compare_series(stock_symbol, "Revenue", "仟元").await?;
    let common_stock_by_year = fetch_compare_series(stock_symbol, "CommonStock", "仟元").await?;

    let mut result = Vec::with_capacity(eps_by_year.len());
    for (year, earnings_per_share) in eps_by_year {
        let metrics = fetch_income_statement_metrics(stock_symbol, year).await;
        result.push(assemble_annual_profit(
            stock_symbol,
            year,
            earnings_per_share,
            revenue_by_year.get(&year).copied(),
            common_stock_by_year.get(&year).copied(),
            metrics.ok(),
        ));
    }

    result.sort_by_key(|item| item.year);
    Ok(result)
}

/// 由單一年度的 EPS、營收、股本與綜合損益表指標組出年度財報，每股值四捨五入到小數兩位。
///
/// 營收或股本缺值時每股營收為 0；綜合損益表抓不到（`None`）時每股稅前淨利為 0。
fn assemble_annual_profit(
    stock_symbol: &str,
    year: i32,
    earnings_per_share: Decimal,
    revenue: Option<Decimal>,
    common_stock: Option<Decimal>,
    metrics: Option<IncomeStatementMetrics>,
) -> share::AnnualProfit {
    let sales_per_share = match (revenue, common_stock) {
        (Some(revenue), Some(common_stock)) => calc_sales_per_share(revenue, common_stock),
        _ => Decimal::ZERO,
    };
    let profit_before_tax = metrics.map_or(Decimal::ZERO, |metrics| {
        let weighted_average_shares =
            calc_weighted_average_shares(metrics.profit_for_eps_total, earnings_per_share);
        calc_profit_before_tax_per_share(metrics.profit_before_tax_total, weighted_average_shares)
    });
    share::AnnualProfit {
        stock_symbol: stock_symbol.to_string(),
        year,
        sales_per_share: sales_per_share.round_dp(2),
        earnings_per_share: earnings_per_share.round_dp(2),
        profit_before_tax: profit_before_tax.round_dp(2),
    }
}

/// 建立 MOPS `compare/data` 查詢所需的表單參數。
///
/// 目前固定查詢年度累計（`Q4`）資料，因此 `qnumber` 固定為 `4`。
fn build_compare_data_params<'a>(
    stock_symbol: &'a str,
    compare_item: &'a str,
    ylabel: &'a str,
) -> HashMap<&'a str, &'a str> {
    let mut params = HashMap::with_capacity(9);
    params.insert("compareItem", compare_item);
    params.insert("quarter", "false");
    params.insert("ylabel", ylabel);
    params.insert("ys", "0");
    params.insert("revenue", "false");
    params.insert("bcodeAvg", "false");
    params.insert("companyAvg", "false");
    params.insert("qnumber", "4");
    params.insert("companyId", stock_symbol);
    params
}

/// 抓取 MOPS 趨勢圖資料並轉為「年度 -> 數值」對照表。
///
/// 此函式用於取得：
/// - `EPS`
/// - `Revenue`
/// - `CommonStock`
async fn fetch_compare_series(
    stock_symbol: &str,
    compare_item: &str,
    ylabel: &str,
) -> Result<HashMap<i32, Decimal>> {
    let url = format!("https://{}/compare/data", HOST);
    let raw = crate::core::util::http::post(
        &url,
        None,
        Some(build_compare_data_params(
            stock_symbol,
            compare_item,
            ylabel,
        )),
    )
    .await?;
    parse_compare_series(&raw, compare_item)
}

/// 抓取指定年度的綜合損益表關鍵指標。
///
/// 目前會回傳：
/// - 稅前淨利總額
/// - 與 EPS 對應的稅後淨利總額
async fn fetch_income_statement_metrics(
    stock_symbol: &str,
    year: i32,
) -> Result<IncomeStatementMetrics> {
    let url = format!("https://{}/compare/report", HOST);
    let ys = format!("{}4", year);
    let mut params = HashMap::with_capacity(9);
    params.insert("compareItem", "IncomeStatement");
    params.insert("quarter", "false");
    params.insert("ylabel", "");
    params.insert("ys", ys.as_str());
    params.insert("revenue", "false");
    params.insert("bcodeAvg", "false");
    params.insert("companyAvg", "false");
    params.insert("qnumber", "");
    params.insert("companyId", stock_symbol);

    let html = crate::core::util::http::post(&url, None, Some(params)).await?;
    parse_income_statement_metrics_from_report(&html)
}

#[async_trait]
impl AnnualProfitFetcher for Mops {
    async fn visit(stock_symbol: &str) -> Result<Vec<share::AnnualProfit>> {
        visit(stock_symbol).await
    }
}

#[cfg(test)]
mod tests;
