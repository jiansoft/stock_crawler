//! # BigGo 三大財報
//!
//! Yahoo 三大財報失敗（例如 HTTP 500）時的備援來源。每檔股票共 9 次請求：
//!
//! | 報表 | endpoint | 期別 |
//! |------|----------|------|
//! | 損益表 | `/stock/statements/profit`＋`/stock/statements/eps` | 單季、年度 |
//! | 現金流量表 | `/stock/statements/cash-flow` | 單季、年度 |
//! | 資產負債表 | `/stock/statements/total-assets`＋`liabilities`＋`equity` | 單季 |
//!
//! ## 回應規則（2026-10 實測，2330／2881／1101／8069／2603／5880 逐期比對）
//!
//! - 金額單位為**元**（JSON 整數），EPS 為元（JSON 小數）；缺值為 `null`，不可當成 0。
//! - `period` 單季為 `"2026 Q2"`，年度為 `"2026"`。
//! - **年度資料含當年度累計**：年中查詢時 `"2026"` 是今年至今的累計（2330 的 2026 EPS
//!   49.33 只是前兩季），不是全年數字，呼叫端要用 [`retain_completed_years`] 濾掉。
//! - 只取與 Yahoo 欄位逐期吻合的項目；應收帳款、短期借款、保留盈餘等定義不同的欄位不取。
//! - 金融業（金控、銀行）的資產負債表與營業利益多為 `null`，照實保留。

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use rust_decimal::{Decimal, prelude::FromPrimitive};
use serde::{Deserialize, Deserializer, de::DeserializeOwned};

use super::{api_symbol, fetch};

/// 同一檔股票的請求之間停頓的時間。
const REQUEST_PAUSE: Duration = Duration::from_millis(400);

/// 報表期別（對應 BigGo 的 `type` 參數）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReportKind {
    /// 單季（`single`）。
    Single,
    /// 年度（`annual`）；當年度為今年至今的累計。
    Annual,
}

impl ReportKind {
    /// BigGo API 使用的 `type` 參數值。
    fn as_param(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Annual => "annual",
        }
    }
}

/// 財報所屬期間。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FiscalPeriod {
    /// 西元年度。
    pub year: i32,
    /// 季別 1～4；年度報表為 `None`。
    pub quarter: Option<u8>,
}

/// 單一期別的財務報表，`T` 為各報表的項目。
#[derive(Debug, Clone, PartialEq)]
pub struct Statement<T> {
    /// 股票代號（不含 `.TW` 後綴）。
    pub stock_symbol: String,
    /// 報表期別。
    pub kind: ReportKind,
    /// 所屬期間。
    pub period: FiscalPeriod,
    /// 報表項目。
    pub items: T,
}

/// 損益表項目（金額單位：元）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IncomeItems {
    /// 營業收入。
    pub revenue: Option<i64>,
    /// 營業毛利。
    pub gross_profit: Option<i64>,
    /// 營業費用。
    pub operating_expenses: Option<i64>,
    /// 營業利益。
    pub operating_income: Option<i64>,
    /// 稅後淨利（含非控制權益，對應 Yahoo 的 `netIncome`）。
    pub income_after_taxes: Option<i64>,
    /// 每股盈餘（元）。
    pub eps: Option<Decimal>,
}

/// 資產負債表項目（金額單位：元）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BalanceItems {
    /// 現金及約當現金。
    pub cash_and_cash_equivalents: Option<i64>,
    /// 透過損益按公允價值衡量之金融資產－流動（對應 Yahoo 的 `shortTermInvestments`）。
    pub current_financial_assets_fvtpl: Option<i64>,
    /// 存貨。
    pub inventories: Option<i64>,
    /// 其他流動資產。
    pub other_current_assets: Option<i64>,
    /// 流動資產。
    pub current_assets: Option<i64>,
    /// 採權益法之投資。
    pub investment_accounted_equity_method: Option<i64>,
    /// 不動產、廠房及設備。
    pub property_plant_equipment: Option<i64>,
    /// 使用權資產。
    pub right_of_use_asset: Option<i64>,
    /// 非流動資產。
    pub noncurrent_assets: Option<i64>,
    /// 資產總額。
    pub total_assets: Option<i64>,
    /// 流動負債。
    pub current_liabilities: Option<i64>,
    /// 應付公司債。
    pub bonds_payable: Option<i64>,
    /// 非流動負債。
    pub noncurrent_liabilities: Option<i64>,
    /// 負債總額。
    pub total_liabilities: Option<i64>,
    /// 股本。
    pub capital_stock: Option<i64>,
    /// 權益總額。
    pub equity: Option<i64>,
}

/// 現金流量表項目（金額單位：元）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CashFlowItems {
    /// 營業活動現金流量。
    pub operating_cash_flow: Option<i64>,
    /// 投資活動現金流量。
    pub investing_cash_flow: Option<i64>,
    /// 籌資活動現金流量。
    pub financing_cash_flow: Option<i64>,
    /// 自由現金流量。
    pub free_cash_flow: Option<i64>,
    /// 本期現金淨增減。
    pub net_cash_flow: Option<i64>,
}

/// 抓取損益表（損益與 EPS 兩支 endpoint 依期別合併）。
///
/// # Errors
///
/// 任一請求失敗、回應格式不符或期別無法解析時回傳錯誤。
pub async fn income_statements(
    stock_symbol: &str,
    kind: ReportKind,
) -> Result<Vec<Statement<IncomeItems>>> {
    let profit: Vec<ProfitRow> =
        fetch_typed("/stock/statements/profit", stock_symbol, kind).await?;
    tokio::time::sleep(REQUEST_PAUSE).await;
    let eps: Vec<EpsRow> = fetch_typed("/stock/statements/eps", stock_symbol, kind).await?;
    merge_income(stock_symbol, kind, profit, eps)
}

/// 抓取現金流量表。
///
/// # Errors
///
/// 請求失敗、回應格式不符或期別無法解析時回傳錯誤。
pub async fn cash_flow_statements(
    stock_symbol: &str,
    kind: ReportKind,
) -> Result<Vec<Statement<CashFlowItems>>> {
    let rows: Vec<CashFlowRow> =
        fetch_typed("/stock/statements/cash-flow", stock_symbol, kind).await?;
    parse_cash_flow(stock_symbol, kind, rows)
}

/// 抓取單季資產負債表（資產、負債、權益三支 endpoint 依期別合併）。
///
/// # Errors
///
/// 任一請求失敗、回應格式不符或期別無法解析時回傳錯誤。
pub async fn balance_sheets(stock_symbol: &str) -> Result<Vec<Statement<BalanceItems>>> {
    let assets: Vec<AssetsRow> = fetch_list("/stock/statements/total-assets", stock_symbol).await?;
    tokio::time::sleep(REQUEST_PAUSE).await;
    let liabilities: Vec<LiabilitiesRow> =
        fetch_list("/stock/statements/liabilities", stock_symbol).await?;
    tokio::time::sleep(REQUEST_PAUSE).await;
    let equity: Vec<EquityRow> = fetch_list("/stock/statements/equity", stock_symbol).await?;
    merge_balance(stock_symbol, assets, liabilities, equity)
}

/// 只保留已有第四季單季資料的年度。
///
/// BigGo 的年度資料包含當年度至今的累計，必須確定該年第四季已公布，年度數字才是全年。
pub fn retain_completed_years<T>(
    annual: Vec<Statement<T>>,
    singles: &[FiscalPeriod],
) -> Vec<Statement<T>> {
    annual
        .into_iter()
        .filter(|statement| {
            singles.contains(&FiscalPeriod {
                year: statement.period.year,
                quarter: Some(4),
            })
        })
        .collect()
}

/// 有 `type` 參數的 endpoint 回應：`{"type": "single", "data": [...]}`。
#[derive(Debug, Deserialize)]
struct Typed<T> {
    /// 各期資料。
    data: Vec<T>,
}

/// 損益 endpoint 的單列。
#[derive(Debug, Deserialize)]
struct ProfitRow {
    period: String,
    #[serde(default, deserialize_with = "amount")]
    revenue: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    gross_profit: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    operating_expenses: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    operating_income: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    income_after_taxes: Option<i64>,
}

/// EPS endpoint 的單列。
#[derive(Debug, Deserialize)]
struct EpsRow {
    period: String,
    #[serde(default)]
    eps: Option<f64>,
}

/// 現金流量 endpoint 的單列。
#[derive(Debug, Deserialize)]
struct CashFlowRow {
    period: String,
    #[serde(default, deserialize_with = "amount")]
    operating_cash_flow: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    investing_cash_flow: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    financing_cash_flow: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    free_cash_flow: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    net_cash_flow: Option<i64>,
}

/// 資產明細 endpoint 的單列（只取與 Yahoo 吻合的欄位）。
#[derive(Debug, Deserialize)]
struct AssetsRow {
    period: String,
    #[serde(default, deserialize_with = "amount")]
    cash_and_cash_equivalents: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    current_financial_assets_fvtpl: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    inventories: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    other_current_assets: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    current_assets: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    investment_accounted_equity_method: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    property_plant_equipment: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    right_of_use_asset: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    noncurrent_assets: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    total_assets: Option<i64>,
}

/// 負債明細 endpoint 的單列（只取與 Yahoo 吻合的欄位）。
#[derive(Debug, Deserialize)]
struct LiabilitiesRow {
    period: String,
    #[serde(default, deserialize_with = "amount")]
    current_liabilities: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    bonds_payable: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    noncurrent_liabilities: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    total_liabilities: Option<i64>,
}

/// 權益明細 endpoint 的單列（只取與 Yahoo 吻合的欄位）。
#[derive(Debug, Deserialize)]
struct EquityRow {
    period: String,
    #[serde(default, deserialize_with = "amount")]
    capital_stock: Option<i64>,
    #[serde(default, deserialize_with = "amount")]
    equity: Option<i64>,
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
async fn fetch_typed<T: DeserializeOwned>(
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
fn typed_rows<T: DeserializeOwned>(data: Option<serde_json::Value>) -> Result<Vec<T>> {
    match data {
        None => Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) if items.is_empty() => Ok(Vec::new()),
        Some(value) => Ok(serde_json::from_value::<Typed<T>>(value)?.data),
    }
}

/// 抓取回傳陣列的 endpoint；查無資料時回空陣列。
async fn fetch_list<T: DeserializeOwned>(path: &str, stock_symbol: &str) -> Result<Vec<T>> {
    let symbol = api_symbol(stock_symbol);
    Ok(fetch(path, &[("stock_id", &symbol)])
        .await?
        .unwrap_or_default())
}

/// 解析 `period`：單季 `"2026 Q2"`、年度 `"2026"`，其餘一律視為異常。
fn parse_period(period: &str, kind: ReportKind) -> Result<FiscalPeriod> {
    let invalid = || anyhow!("Unexpected BigGo period {period:?} for {kind:?}");
    let parse_year = |year: &str| -> Result<i32> {
        if year.len() != 4 {
            return Err(invalid());
        }
        year.parse().map_err(|_| invalid())
    };
    match kind {
        ReportKind::Single => {
            let (year, quarter) = period.split_once(" Q").ok_or_else(invalid)?;
            let quarter: u8 = quarter.parse().map_err(|_| invalid())?;
            if !(1..=4).contains(&quarter) {
                return Err(invalid());
            }
            Ok(FiscalPeriod {
                year: parse_year(year)?,
                quarter: Some(quarter),
            })
        }
        ReportKind::Annual => Ok(FiscalPeriod {
            year: parse_year(period)?,
            quarter: None,
        }),
    }
}

/// 依期別合併損益與 EPS；任一邊缺某期時該期仍保留，缺的項目為 `None`。
fn merge_income(
    stock_symbol: &str,
    kind: ReportKind,
    profit: Vec<ProfitRow>,
    eps: Vec<EpsRow>,
) -> Result<Vec<Statement<IncomeItems>>> {
    let mut merged: BTreeMap<FiscalPeriod, IncomeItems> = BTreeMap::new();
    for row in profit {
        let items = merged.entry(parse_period(&row.period, kind)?).or_default();
        items.revenue = row.revenue;
        items.gross_profit = row.gross_profit;
        items.operating_expenses = row.operating_expenses;
        items.operating_income = row.operating_income;
        items.income_after_taxes = row.income_after_taxes;
    }
    for row in eps {
        let period = parse_period(&row.period, kind)?;
        let eps = row
            .eps
            .map(|value| {
                Decimal::from_f64(value)
                    .map(|value| value.round_dp(4))
                    .ok_or_else(|| anyhow!("EPS {value} for {} is out of range", row.period))
            })
            .transpose()?;
        merged.entry(period).or_default().eps = eps;
    }
    Ok(into_statements(stock_symbol, kind, merged))
}

/// 解析現金流量表。
fn parse_cash_flow(
    stock_symbol: &str,
    kind: ReportKind,
    rows: Vec<CashFlowRow>,
) -> Result<Vec<Statement<CashFlowItems>>> {
    let mut merged = BTreeMap::new();
    for row in rows {
        merged.insert(
            parse_period(&row.period, kind)?,
            CashFlowItems {
                operating_cash_flow: row.operating_cash_flow,
                investing_cash_flow: row.investing_cash_flow,
                financing_cash_flow: row.financing_cash_flow,
                free_cash_flow: row.free_cash_flow,
                net_cash_flow: row.net_cash_flow,
            },
        );
    }
    Ok(into_statements(stock_symbol, kind, merged))
}

/// 依期別合併資產、負債、權益三份明細。
fn merge_balance(
    stock_symbol: &str,
    assets: Vec<AssetsRow>,
    liabilities: Vec<LiabilitiesRow>,
    equity: Vec<EquityRow>,
) -> Result<Vec<Statement<BalanceItems>>> {
    let kind = ReportKind::Single;
    let mut merged: BTreeMap<FiscalPeriod, BalanceItems> = BTreeMap::new();
    for row in assets {
        let items = merged.entry(parse_period(&row.period, kind)?).or_default();
        items.cash_and_cash_equivalents = row.cash_and_cash_equivalents;
        items.current_financial_assets_fvtpl = row.current_financial_assets_fvtpl;
        items.inventories = row.inventories;
        items.other_current_assets = row.other_current_assets;
        items.current_assets = row.current_assets;
        items.investment_accounted_equity_method = row.investment_accounted_equity_method;
        items.property_plant_equipment = row.property_plant_equipment;
        items.right_of_use_asset = row.right_of_use_asset;
        items.noncurrent_assets = row.noncurrent_assets;
        items.total_assets = row.total_assets;
    }
    for row in liabilities {
        let items = merged.entry(parse_period(&row.period, kind)?).or_default();
        items.current_liabilities = row.current_liabilities;
        items.bonds_payable = row.bonds_payable;
        items.noncurrent_liabilities = row.noncurrent_liabilities;
        items.total_liabilities = row.total_liabilities;
    }
    for row in equity {
        let items = merged.entry(parse_period(&row.period, kind)?).or_default();
        items.capital_stock = row.capital_stock;
        items.equity = row.equity;
    }
    Ok(into_statements(stock_symbol, kind, merged))
}

/// 把依期別合併好的項目轉成 [`Statement`]，期別由舊到新。
fn into_statements<T>(
    stock_symbol: &str,
    kind: ReportKind,
    merged: BTreeMap<FiscalPeriod, T>,
) -> Vec<Statement<T>> {
    merged
        .into_iter()
        .map(|(period, items)| Statement {
            stock_symbol: stock_symbol.to_string(),
            kind,
            period,
            items,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;
    use serde::de::DeserializeOwned;

    use super::*;
    use crate::infra::crawler::biggo::Envelope;

    /// 解析 fixture 的回應外層並取出 `data`。
    fn data<T: DeserializeOwned>(json: &str) -> T {
        let envelope: Envelope<T> = serde_json::from_str(json).expect("fixture should parse");
        assert!(envelope.result);
        envelope.data.expect("fixture should have data")
    }

    fn period(year: i32, quarter: Option<u8>) -> FiscalPeriod {
        FiscalPeriod { year, quarter }
    }

    #[test]
    fn parse_period_accepts_only_known_formats() {
        assert_eq!(
            parse_period("2026 Q2", ReportKind::Single).unwrap(),
            period(2026, Some(2))
        );
        assert_eq!(
            parse_period("2025", ReportKind::Annual).unwrap(),
            period(2025, None)
        );
        for (raw, kind) in [
            ("2026Q2", ReportKind::Single),
            ("2026 Q5", ReportKind::Single),
            ("26 Q1", ReportKind::Single),
            ("2026", ReportKind::Single),
            ("2026 Q2", ReportKind::Annual),
            ("abcd", ReportKind::Annual),
        ] {
            assert!(
                parse_period(raw, kind).is_err(),
                "{raw} ({kind:?}) 應被拒收"
            );
        }
    }

    /// 2330 2026Q2：損益與 EPS 依期別合併，數字與官方財報（及 Yahoo）一致。
    #[test]
    fn merge_income_joins_profit_and_eps_by_period() {
        let profit: Typed<ProfitRow> = data(include_str!("testdata/profit_single_2330.json"));
        let eps: Typed<EpsRow> = data(include_str!("testdata/eps_single_2330.json"));
        let statements =
            merge_income("2330", ReportKind::Single, profit.data, eps.data).expect("merge");

        assert_eq!(statements.len(), 3);
        let latest = statements.last().expect("latest");
        assert_eq!(latest.stock_symbol, "2330");
        assert_eq!(latest.period, period(2026, Some(2)));
        assert_eq!(latest.items.revenue, Some(1_270_380_250_000));
        assert_eq!(latest.items.gross_profit, Some(860_310_695_000));
        assert_eq!(latest.items.operating_expenses, Some(98_982_083_000));
        assert_eq!(latest.items.operating_income, Some(766_602_651_000));
        assert_eq!(latest.items.income_after_taxes, Some(706_780_923_000));
        assert_eq!(latest.items.eps, Some(dec!(27.25)));
    }

    /// 只有 EPS 的期別仍保留，缺的損益項目為 `None`。
    #[test]
    fn merge_income_keeps_periods_present_on_one_side() {
        let eps = vec![EpsRow {
            period: "2026 Q3".to_string(),
            eps: Some(1.5),
        }];
        let statements = merge_income("2330", ReportKind::Single, Vec::new(), eps).expect("merge");
        assert_eq!(statements.len(), 1);
        assert_eq!(statements[0].items.revenue, None);
        assert_eq!(statements[0].items.eps, Some(dec!(1.5)));
    }

    /// 年度資料的當年度是今年至今的累計，沒有第四季單季資料的年度要濾掉。
    #[test]
    fn retain_completed_years_drops_the_year_to_date_total() {
        let profit: Typed<ProfitRow> = data(include_str!("testdata/profit_annual_2330.json"));
        let eps: Typed<EpsRow> = data(include_str!("testdata/eps_annual_2330.json"));
        let annual = merge_income("2330", ReportKind::Annual, profit.data, eps.data).unwrap();
        assert_eq!(annual.len(), 2, "fixture 含 2026（累計）與 2025");

        let singles = [
            period(2026, Some(2)),
            period(2026, Some(1)),
            period(2025, Some(4)),
        ];
        let completed = retain_completed_years(annual, &singles);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].period, period(2025, None));
        assert_eq!(completed[0].items.eps, Some(dec!(66.26)));
        assert_eq!(completed[0].items.revenue, Some(3_809_054_272_000));
    }

    #[test]
    fn parse_cash_flow_reads_all_items() {
        let rows: Typed<CashFlowRow> = data(include_str!("testdata/cash_flow_single_2330.json"));
        let statements = parse_cash_flow("2330", ReportKind::Single, rows.data).unwrap();
        let latest = statements.last().expect("latest");
        assert_eq!(latest.period, period(2026, Some(2)));
        assert_eq!(latest.items.operating_cash_flow, Some(783_364_977_000));
        assert_eq!(latest.items.investing_cash_flow, Some(-492_810_418_000));
        assert_eq!(latest.items.financing_cash_flow, Some(-184_653_221_000));
        assert_eq!(latest.items.free_cash_flow, Some(290_554_559_000));
        assert_eq!(latest.items.net_cash_flow, Some(98_580_985_000));

        let annual: Typed<CashFlowRow> = data(include_str!("testdata/cash_flow_annual_2330.json"));
        let annual = parse_cash_flow("2330", ReportKind::Annual, annual.data).unwrap();
        assert_eq!(annual.last().unwrap().period, period(2025, None));
        assert_eq!(
            annual.last().unwrap().items.operating_cash_flow,
            Some(2_274_975_625_000)
        );
    }

    /// 資產、負債、權益三份明細依期別合併成一張資產負債表。
    #[test]
    fn merge_balance_joins_three_breakdowns() {
        let assets: Vec<AssetsRow> = data(include_str!("testdata/total_assets_2330.json"));
        let liabilities: Vec<LiabilitiesRow> = data(include_str!("testdata/liabilities_2330.json"));
        let equity: Vec<EquityRow> = data(include_str!("testdata/equity_2330.json"));
        let statements = merge_balance("2330", assets, liabilities, equity).unwrap();

        assert_eq!(statements.len(), 2);
        let latest = &statements.last().unwrap().items;
        assert_eq!(latest.cash_and_cash_equivalents, Some(3_134_218_213_000));
        assert_eq!(latest.current_financial_assets_fvtpl, Some(226_375_000));
        assert_eq!(latest.inventories, Some(385_524_542_000));
        assert_eq!(latest.current_assets, Some(4_565_700_742_000));
        assert_eq!(
            latest.investment_accounted_equity_method,
            Some(18_126_371_000)
        );
        assert_eq!(latest.property_plant_equipment, Some(4_302_880_478_000));
        assert_eq!(latest.total_assets, Some(9_375_654_727_000));
        assert_eq!(latest.current_liabilities, Some(1_857_761_825_000));
        assert_eq!(latest.bonds_payable, Some(815_036_716_000));
        assert_eq!(latest.total_liabilities, Some(2_901_183_746_000));
        assert_eq!(latest.capital_stock, Some(259_323_701_000));
        assert_eq!(latest.equity, Some(6_474_470_981_000));
    }

    /// 查無資料時 BigGo 回 `[]` 或 `null`，都視為空；非空陣列不是預期格式，要拒收。
    #[test]
    fn typed_rows_treats_empty_array_as_no_data() {
        let empty: Vec<EpsRow> = typed_rows(Some(serde_json::json!([]))).unwrap();
        assert!(empty.is_empty());
        let none: Vec<EpsRow> = typed_rows(None).unwrap();
        assert!(none.is_empty());

        let rows: Vec<EpsRow> = typed_rows(Some(serde_json::json!({
            "type": "single",
            "data": [{"period": "2026 Q2", "eps": 1.2}]
        })))
        .unwrap();
        assert_eq!(rows.len(), 1);

        let unexpected: Result<Vec<EpsRow>> =
            typed_rows(Some(serde_json::json!([{"period": "2026 Q2"}])));
        assert!(unexpected.is_err());
    }

    /// 金額接受整數與小數部分為 0 的數字，`null` 為 `None`，有小數一律拒收。
    #[test]
    fn amount_accepts_only_integral_numbers() {
        let parse = |json: &str| serde_json::from_str::<CashFlowRow>(json);
        let row =
            parse(r#"{"period":"2026 Q2","operating_cash_flow":1000.0,"net_cash_flow":null}"#)
                .unwrap();
        assert_eq!(row.operating_cash_flow, Some(1000));
        assert_eq!(row.net_cash_flow, None);
        assert_eq!(row.free_cash_flow, None, "缺欄位為 None");
        assert!(parse(r#"{"period":"2026 Q2","operating_cash_flow":1000.5}"#).is_err());
        assert!(parse(r#"{"period":"2026 Q2","operating_cash_flow":"1000"}"#).is_err());
    }
}
