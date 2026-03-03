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

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rust_decimal::Decimal;
use scraper::{Html, Selector};
use serde::Deserialize;

use crate::{
    crawler::{
        mops::HOST,
        share::{self, AnnualProfitFetcher},
    },
    util::text,
};

/// MOPS 年度財報抓取器標記型別。
pub struct Mops {}

/// MOPS 趨勢圖 API 回應。
#[derive(Deserialize, Debug)]
struct CompareDataResponse {
    /// X 軸時間點，例如 `2024Q4`。
    #[serde(rename = "xaxisList")]
    xaxis_list: Vec<String>,
    /// 各家公司折線資料。
    #[serde(rename = "graphData")]
    graph_data: Vec<GraphSeries>,
}

/// 單一公司的趨勢資料序列。
#[derive(Deserialize, Debug)]
struct GraphSeries {
    /// 每個資料點，格式為 `[x_index, value, source_flag]`。
    data: Vec<(usize, Option<f64>, String)>,
}

/// MOPS 綜合損益表中本次需要的年度指標。
///
/// 這些欄位都直接來自 `compare/report` 的綜合損益表，
/// 單位皆為「仟元」。
struct IncomeStatementMetrics {
    /// 稅前淨利總額（仟元）。
    profit_before_tax_total: Decimal,
    /// 用來對應 EPS 的稅後淨利總額（仟元）。
    profit_for_eps_total: Decimal,
}

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
        let sales_per_share = match (
            revenue_by_year.get(&year).copied(),
            common_stock_by_year.get(&year).copied(),
        ) {
            (Some(revenue), Some(common_stock)) => calc_sales_per_share(revenue, common_stock),
            _ => Decimal::ZERO,
        };

        let profit_before_tax = match fetch_income_statement_metrics(stock_symbol, year).await {
            Ok(metrics) => {
                let weighted_average_shares =
                    calc_weighted_average_shares(metrics.profit_for_eps_total, earnings_per_share);
                calc_profit_before_tax_per_share(
                    metrics.profit_before_tax_total,
                    weighted_average_shares,
                )
            }
            Err(_) => Decimal::ZERO,
        };

        result.push(share::AnnualProfit {
            stock_symbol: stock_symbol.to_string(),
            year,
            sales_per_share: sales_per_share.round_dp(2),
            earnings_per_share: earnings_per_share.round_dp(2),
            profit_before_tax: profit_before_tax.round_dp(2),
        });
    }

    result.sort_by_key(|item| item.year);
    Ok(result)
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
    let raw = crate::util::http::post(
        &url,
        None,
        Some(build_compare_data_params(
            stock_symbol,
            compare_item,
            ylabel,
        )),
    )
    .await?;
    let response: CompareDataResponse = serde_json::from_str(&raw).map_err(|why| {
        anyhow!(
            "Failed to parse MOPS compare/data response for {} because {:?}. body={}",
            compare_item,
            why,
            raw
        )
    })?;

    let series = response
        .graph_data
        .first()
        .ok_or_else(|| anyhow!("Failed to find graphData for MOPS {}", compare_item))?;

    let mut result = HashMap::with_capacity(series.data.len());
    for (x_index, value, _) in &series.data {
        let Some(raw_value) = value else {
            continue;
        };

        let xaxis = response.xaxis_list.get(*x_index).ok_or_else(|| {
            anyhow!(
                "Failed to find xaxis index {} for {}",
                x_index,
                compare_item
            )
        })?;
        let year = parse_year_from_xaxis(xaxis)?;
        let decimal = Decimal::from_f64_retain(*raw_value).ok_or_else(|| {
            anyhow!(
                "Failed to convert MOPS {} value {} to decimal",
                compare_item,
                raw_value
            )
        })?;
        result.insert(year, decimal);
    }

    Ok(result)
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

    let html = crate::util::http::post(&url, None, Some(params)).await?;
    parse_income_statement_metrics_from_report(&html)
}

/// 從 MOPS X 軸標籤中解析年度。
///
/// 例如：
/// - `2024Q4` -> `2024`
fn parse_year_from_xaxis(xaxis: &str) -> Result<i32> {
    let year = xaxis
        .get(..4)
        .ok_or_else(|| anyhow!("Failed to parse year from xaxis '{}'", xaxis))?;

    year.parse::<i32>()
        .map_err(|why| anyhow!("Failed to parse xaxis year '{}' because {:?}", xaxis, why))
}

/// 以 MOPS 的 `Revenue` 與 `CommonStock` 推算每股營收。
///
/// # 單位
/// - `revenue`：仟元
/// - `common_stock`：仟元
///
/// # 公式
/// `Revenue * 10 / CommonStock`
fn calc_sales_per_share(revenue: Decimal, common_stock: Decimal) -> Decimal {
    if common_stock.is_zero() {
        return Decimal::ZERO;
    }

    // `Revenue` 與 `CommonStock` 皆為仟元；股本面額預設以 10 元換算為股數。
    revenue * Decimal::from(10) / common_stock
}

/// 由稅後淨利與稅後 EPS 反推加權平均股數。
///
/// # 單位
/// - `net_profit_total`：仟元
/// - `earnings_per_share`：元
///
/// # 回傳
/// - 加權平均股數（仟股）
///
/// # 公式
/// `weighted_average_shares = net_profit_total / earnings_per_share`
fn calc_weighted_average_shares(net_profit_total: Decimal, earnings_per_share: Decimal) -> Decimal {
    if earnings_per_share.is_zero() {
        return Decimal::ZERO;
    }

    // `net_profit_total` 為仟元、`earnings_per_share` 為元，
    // 因此可直接反推「加權平均股數（仟股）」。
    net_profit_total / earnings_per_share
}

/// 將稅前淨利總額換算成每股稅前淨利。
///
/// # 單位
/// - `total_profit_before_tax`：仟元
/// - `weighted_average_shares`：仟股
///
/// # 回傳
/// - 每股稅前淨利（元）
///
/// # 公式
/// `profit_before_tax_per_share = total_profit_before_tax / weighted_average_shares`
fn calc_profit_before_tax_per_share(
    total_profit_before_tax: Decimal,
    weighted_average_shares: Decimal,
) -> Decimal {
    if weighted_average_shares.is_zero() {
        return Decimal::ZERO;
    }

    // `total_profit_before_tax` 為仟元，`weighted_average_shares` 為仟股，
    // 兩者相除後即為每股稅前淨利（元）。
    total_profit_before_tax / weighted_average_shares
}

/// 從 MOPS 綜合損益表 HTML 解析本次需要的年度指標。
///
/// 會同時支援一般產業與金融股的欄位名稱差異。
/// 若找不到對應列，則該指標會以 `0` 回填。
fn parse_income_statement_metrics_from_report(html: &str) -> Result<IncomeStatementMetrics> {
    const PROFIT_BEFORE_TAX_LABELS: [&str; 3] = [
        "稅前淨利（淨損）",
        "繼續營業單位稅前損益",
        "繼續營業單位稅前淨利（淨損）",
    ];
    const NET_PROFIT_LABELS: [&str; 3] = [
        "本期淨利（淨損）",
        "本期稅後淨利（淨損）",
        "繼續營業單位本期淨利（淨損）",
    ];
    const PROFIT_FOR_EPS_LABELS: [&str; 2] = ["母公司業主（淨利∕損）", "母公司業主（淨利／損）"];

    let document = Html::parse_document(html);
    let label_selector = Selector::parse("#headTable tbody tr td")
        .map_err(|why| anyhow!("Failed to build head selector because {:?}", why))?;
    let value_selector = Selector::parse("#bodyTable tbody tr td")
        .map_err(|why| anyhow!("Failed to build body selector because {:?}", why))?;

    let labels = document
        .select(&label_selector)
        .map(|node| node.text().collect::<String>().trim().to_string())
        .collect::<Vec<_>>();
    let values = document
        .select(&value_selector)
        .map(|node| node.text().collect::<String>().trim().to_string())
        .collect::<Vec<_>>();

    let mut profit_before_tax_total = None;
    let mut net_profit_total = None;
    let mut profit_for_eps_total = None;

    for (label, value) in labels.iter().zip(values.iter()) {
        if PROFIT_BEFORE_TAX_LABELS.contains(&label.as_str()) {
            profit_before_tax_total = Some(text::parse_decimal(value, None).map_err(|why| {
                anyhow!(
                    "Failed to parse MOPS profit before tax '{}' because {:?}",
                    value,
                    why
                )
            })?);
        }

        if NET_PROFIT_LABELS.contains(&label.as_str()) {
            net_profit_total = Some(text::parse_decimal(value, None).map_err(|why| {
                anyhow!(
                    "Failed to parse MOPS net profit '{}' because {:?}",
                    value,
                    why
                )
            })?);
        }

        if PROFIT_FOR_EPS_LABELS.contains(&label.as_str()) {
            profit_for_eps_total = Some(text::parse_decimal(value, None).map_err(|why| {
                anyhow!(
                    "Failed to parse MOPS profit for EPS '{}' because {:?}",
                    value,
                    why
                )
            })?);
        }
    }

    Ok(IncomeStatementMetrics {
        profit_before_tax_total: profit_before_tax_total.unwrap_or(Decimal::ZERO),
        profit_for_eps_total: profit_for_eps_total
            .or(net_profit_total)
            .unwrap_or(Decimal::ZERO),
    })
}

#[async_trait]
impl AnnualProfitFetcher for Mops {
    async fn visit(stock_symbol: &str) -> Result<Vec<share::AnnualProfit>> {
        visit(stock_symbol).await
    }
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[test]
    fn test_parse_year_from_xaxis() {
        assert_eq!(parse_year_from_xaxis("2024Q4").unwrap(), 2024);
    }

    #[test]
    fn test_calc_sales_per_share() {
        let revenue = Decimal::from_str_exact("2894307699").unwrap();
        let common_stock = Decimal::from_str_exact("259327332").unwrap();

        assert_eq!(
            calc_sales_per_share(revenue, common_stock).round_dp(2),
            Decimal::from_str_exact("111.61").unwrap()
        );
    }

    #[test]
    fn test_round_per_share_values_to_two_decimal_places() {
        let sales_per_share = Decimal::from_str_exact("4.6650656274625312897574062788")
            .unwrap()
            .round_dp(2);
        let earnings_per_share = Decimal::from_str_exact("1.1599999999999999200639422265")
            .unwrap()
            .round_dp(2);

        assert_eq!(sales_per_share, Decimal::from_str_exact("4.67").unwrap());
        assert_eq!(earnings_per_share, Decimal::from_str_exact("1.16").unwrap());
    }

    #[test]
    fn test_calc_weighted_average_shares() {
        let profit_for_eps_total = Decimal::from_str_exact("1717882.627").unwrap();
        let earnings_per_share = Decimal::from_str_exact("66.26").unwrap();

        assert_eq!(
            calc_weighted_average_shares(profit_for_eps_total, earnings_per_share).round_dp(3),
            Decimal::from_str_exact("25926.390").unwrap()
        );
    }

    #[test]
    fn test_calc_profit_before_tax_per_share() {
        let total_profit_before_tax = Decimal::from_str_exact("2041663").unwrap();
        let weighted_average_shares = Decimal::from_str_exact("25926.390").unwrap();

        assert_eq!(
            calc_profit_before_tax_per_share(total_profit_before_tax, weighted_average_shares)
                .round_dp(2),
            Decimal::from_str_exact("78.75").unwrap()
        );
    }

    #[test]
    fn test_parse_income_statement_metrics_from_report() {
        let html = r#"
            <div id="headTable">
                <table>
                    <tbody>
                        <tr><td>營業收入合計</td></tr>
                        <tr><td>稅前淨利（淨損）</td></tr>
                        <tr><td>本期淨利（淨損）</td></tr>
                    </tbody>
                </table>
            </div>
            <div id="bodyTable">
                <table>
                    <tbody>
                        <tr><td>3,809,054,272</td></tr>
                        <tr><td>1,700,000,000</td></tr>
                        <tr><td>1,400,000,000</td></tr>
                    </tbody>
                </table>
            </div>
        "#;

        let metrics = parse_income_statement_metrics_from_report(html).unwrap();

        assert_eq!(
            metrics.profit_before_tax_total,
            Decimal::from_str_exact("1700000000").unwrap()
        );
        assert_eq!(
            metrics.profit_for_eps_total,
            Decimal::from_str_exact("1400000000").unwrap()
        );
    }

    #[test]
    fn test_parse_income_statement_metrics_from_financial_report() {
        let html = r#"
            <div id="headTable">
                <table>
                    <tbody>
                        <tr><td>淨收益</td></tr>
                        <tr><td>繼續營業單位稅前淨利（淨損）</td></tr>
                        <tr><td>本期稅後淨利（淨損）</td></tr>
                    </tbody>
                </table>
            </div>
            <div id="bodyTable">
                <table>
                    <tbody>
                        <tr><td>60,000,000</td></tr>
                        <tr><td>18,765,432</td></tr>
                        <tr><td>15,000,000</td></tr>
                    </tbody>
                </table>
            </div>
        "#;

        let metrics = parse_income_statement_metrics_from_report(html).unwrap();

        assert_eq!(
            metrics.profit_before_tax_total,
            Decimal::from_str_exact("18765432").unwrap()
        );
        assert_eq!(
            metrics.profit_for_eps_total,
            Decimal::from_str_exact("15000000").unwrap()
        );
    }

    #[test]
    fn test_parse_income_statement_metrics_from_empty_report() {
        let html = r#"
            <div id="headTable">
                <table><tbody></tbody></table>
            </div>
            <div id="bodyTable">
                <table><tbody></tbody></table>
            </div>
        "#;

        let metrics = parse_income_statement_metrics_from_report(html).unwrap();

        assert_eq!(metrics.profit_before_tax_total, Decimal::ZERO);
        assert_eq!(metrics.profit_for_eps_total, Decimal::ZERO);
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 mops::annual_profit::visit".to_string());

        for stock_symbol in ["2330", "2838"] {
            match visit(stock_symbol).await {
                Ok(result) => {
                    dbg!(&result);
                    logging::debug_file_async(format!("mops : {:#?}", result));
                }
                Err(why) => {
                    logging::debug_file_async(format!(
                        "Failed to mops::annual_profit::visit({}) because {:?}",
                        stock_symbol, why
                    ));
                    panic!(
                        "mops::annual_profit::visit({}) failed: {:?}",
                        stock_symbol, why
                    );
                }
            }
        }

        logging::debug_file_async("結束 mops::annual_profit::visit".to_string());
    }
}
