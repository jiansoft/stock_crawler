//! BigGo 三大財報 endpoint 的回應資料列（金額缺值為 `None`）與讀取。

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, de::DeserializeOwned};

use super::super::{api_symbol, fetch};
use super::ReportKind;

/// 有 `type` 參數的 endpoint 回應：`{"type": "single", "data": [...]}`。
#[derive(Debug, Deserialize)]
pub(super) struct Typed<T> {
    /// 各期資料。
    pub(super) data: Vec<T>,
}

/// 損益 endpoint 的單列。
#[derive(Debug, Deserialize)]
pub(super) struct ProfitRow {
    pub(super) period: String,
    #[serde(default, deserialize_with = "amount")]
    pub(super) revenue: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) gross_profit: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) operating_expenses: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) operating_income: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) income_after_taxes: Option<i64>,
}

/// EPS endpoint 的單列。
#[derive(Debug, Deserialize)]
pub(super) struct EpsRow {
    pub(super) period: String,
    #[serde(default)]
    pub(super) eps: Option<f64>,
}

/// 現金流量 endpoint 的單列。
#[derive(Debug, Deserialize)]
pub(super) struct CashFlowRow {
    pub(super) period: String,
    #[serde(default, deserialize_with = "amount")]
    pub(super) operating_cash_flow: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) investing_cash_flow: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) financing_cash_flow: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) free_cash_flow: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) net_cash_flow: Option<i64>,
}

/// 資產明細 endpoint 的單列（只取與 Yahoo 吻合的欄位）。
#[derive(Debug, Deserialize)]
pub(super) struct AssetsRow {
    pub(super) period: String,
    #[serde(default, deserialize_with = "amount")]
    pub(super) cash_and_cash_equivalents: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) current_financial_assets_fvtpl: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) inventories: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) other_current_assets: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) current_assets: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) investment_accounted_equity_method: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) property_plant_equipment: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) right_of_use_asset: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) noncurrent_assets: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) total_assets: Option<i64>,
}

/// 負債明細 endpoint 的單列（只取與 Yahoo 吻合的欄位）。
#[derive(Debug, Deserialize)]
pub(super) struct LiabilitiesRow {
    pub(super) period: String,
    #[serde(default, deserialize_with = "amount")]
    pub(super) current_liabilities: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) bonds_payable: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) noncurrent_liabilities: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) total_liabilities: Option<i64>,
}

/// 權益明細 endpoint 的單列（只取與 Yahoo 吻合的欄位）。
#[derive(Debug, Deserialize)]
pub(super) struct EquityRow {
    pub(super) period: String,
    #[serde(default, deserialize_with = "amount")]
    pub(super) capital_stock: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    pub(super) equity: Option<i64>,
}

/// 金額欄位：接受 JSON 整數，或小數部分為 0 的數字；`null` 為 `None`。
///
/// 金額超過 2^53 的浮點數無法確保精確，一律拒收。
fn amount<'de, D>(deserializer: D) -> std::result::Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    const MAX_EXACT_FLOAT: f64 = 9_007_199_254_740_992.0;
    let Some(number) = Option::<serde_json::Number>::deserialize(deserializer)? else {
        return Ok(None);
    };
    if let Some(value) = number.as_i64() {
        return Ok(Some(value));
    }
    match number.as_f64() {
        Some(value) if value.fract() == 0.0 && value.abs() <= MAX_EXACT_FLOAT => {
            Ok(Some(value as i64))
        }
        _ => Err(serde::de::Error::custom(format!(
            "amount {number} is not an integer"
        ))),
    }
}

/// 抓取有 `type` 參數的 endpoint；查無資料時回空陣列。
pub(super) async fn fetch_typed<T: DeserializeOwned>(
    path: &str,
    stock_symbol: &str,
    kind: ReportKind,
) -> Result<Vec<T>> {
    let symbol = api_symbol(stock_symbol);
    let data: Option<serde_json::Value> =
        fetch(path, &[("stock_id", &symbol), ("type", kind.as_param())]).await?;
    typed_rows(data).with_context(|| format!("Unexpected BigGo {path} data for {symbol}"))
}

/// 取出有 `type` 參數的 endpoint 的資料列。
///
/// 有資料時是 `{"type": "single", "data": [...]}`；查無資料時 BigGo 回空陣列 `[]`
/// （3718 中光電投控 2026-10 實測）或 `null`，兩者都視為沒有資料。
pub(super) fn typed_rows<T: DeserializeOwned>(data: Option<serde_json::Value>) -> Result<Vec<T>> {
    match data {
        None => Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) if items.is_empty() => Ok(Vec::new()),
        Some(value) => Ok(serde_json::from_value::<Typed<T>>(value)?.data),
    }
}

/// 抓取回傳陣列的 endpoint；查無資料時回空陣列。
pub(super) async fn fetch_list<T: DeserializeOwned>(
    path: &str,
    stock_symbol: &str,
) -> Result<Vec<T>> {
    let symbol = api_symbol(stock_symbol);
    Ok(fetch(path, &[("stock_id", &symbol)])
        .await?
        .unwrap_or_default())
}
