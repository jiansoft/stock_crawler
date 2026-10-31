//! MOPS 年度財報回應的解析與每股數值換算：`compare/data` 的 JSON、`compare/report` 的綜合損益表 HTML，
//! 以及每股營收、加權平均股數與每股稅前淨利的公式（說明見上層模組）。純計算，不做 I/O。

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use rust_decimal::Decimal;
use scraper::{Html, Selector};
use serde::Deserialize;

use crate::core::util::text;

/// MOPS 趨勢圖 API 回應。
#[derive(Deserialize, Debug)]
pub(super) struct CompareDataResponse {
    /// X 軸時間點，例如 `2024Q4`。
    #[serde(rename = "xaxisList")]
    xaxis_list: Vec<String>,
    /// 各家公司折線資料。
    #[serde(rename = "graphData")]
    graph_data: Vec<GraphSeries>,
}

/// 單一公司的趨勢資料序列。
#[derive(Deserialize, Debug)]
pub(super) struct GraphSeries {
    /// 每個資料點，格式為 `[x_index, value, source_flag]`。
    data: Vec<(usize, Option<f64>, String)>,
}

/// MOPS 綜合損益表中本次需要的年度指標。
///
/// 這些欄位都直接來自 `compare/report` 的綜合損益表，
/// 單位皆為「仟元」。
pub(super) struct IncomeStatementMetrics {
    /// 稅前淨利總額（仟元）。
    pub(super) profit_before_tax_total: Decimal,
    /// 用來對應 EPS 的稅後淨利總額（仟元）。
    pub(super) profit_for_eps_total: Decimal,
}

/// 把 MOPS `compare/data` 的 JSON 轉為「年度 -> 數值」對照表；值為 `null` 的資料點略過。
pub(super) fn parse_compare_series(raw: &str, compare_item: &str) -> Result<HashMap<i32, Decimal>> {
    let response: CompareDataResponse = serde_json::from_str(raw).map_err(|why| {
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

/// 從 MOPS X 軸標籤中解析年度。
///
/// 例如：
/// - `2024Q4` -> `2024`
pub(super) fn parse_year_from_xaxis(xaxis: &str) -> Result<i32> {
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
pub(super) fn calc_sales_per_share(revenue: Decimal, common_stock: Decimal) -> Decimal {
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
pub(super) fn calc_weighted_average_shares(
    net_profit_total: Decimal,
    earnings_per_share: Decimal,
) -> Decimal {
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
pub(super) fn calc_profit_before_tax_per_share(
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
pub(super) fn parse_income_statement_metrics_from_report(
    html: &str,
) -> Result<IncomeStatementMetrics> {
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
