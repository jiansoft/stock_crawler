//! `/stocks/{symbol}/financial-reports` handler：單一股票的三大財務報表。
//!
//! 資料來自 `income_statement`、`balance_sheet`、`cash_flow_statement`（由
//! `app::backfill::financial_report` 排程寫入）。三張表的主鍵都含 `source`：
//! Yahoo 失敗時以 BigGo 備援寫入 `source = 'biggo'`，與 Yahoo 的列並存，
//! 因此同一期兩者都有時只取 Yahoo（欄位較完整）。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::{analytical_decimal_to_f64, database_error, ensure_stock_exists, error_response};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    AMOUNT_UNIT, BalanceSheetItem, CashFlowStatementItem, ErrorBody, FinancialReportParams,
    FinancialReportResponse, IncomeStatementItem,
};

/// 將 API 的期間類型轉成資料表 `period_type`；不支援的值回 `None`。
///
/// 資產負債表只有 `single`，`annual` 時改取第 4 季季末，由 SQL 另行處理。
fn db_period_type(period_type: &str) -> Option<&'static str> {
    match period_type {
        "quarterly" => Some("single"),
        "annual" => Some("annual"),
        _ => None,
    }
}

/// 以 `YYYY-Qn`／`YYYY-A` 表示一期報表。
fn period_label(fiscal_year: i32, quarter: &str) -> String {
    format!("{fiscal_year}-{quarter}")
}

/// 查詢單一股票的三大財務報表（損益表、資產負債表、現金流量表）。
///
/// `quarterly` 回傳單季資料；`annual` 回傳年度損益與現金流量，資產負債表是
/// 時點數值、資料表沒有年度列，改回傳各年第 4 季季末。`limit` 分別套用在
/// 三張報表上。
///
/// # Errors
///
/// 參數不合法回 422、股票不存在回 404、驗證失敗回 401；資料庫查詢失敗
/// 時記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/stocks/{symbol}/financial-reports", tag = "data-api", params(("symbol" = String, Path, description = "股票代號"), FinancialReportParams), responses((status = 200, body = FinancialReportResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn financial_reports(
    Path(symbol): Path<String>,
    Query(params): Query<FinancialReportParams>,
) -> Response {
    let period_type = params.period_type.as_deref().unwrap_or("quarterly");
    let Some(db_period) = db_period_type(period_type) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "period_type 必須為 quarterly 或 annual",
        );
    };
    let limit = params.limit.unwrap_or(8);
    if !(1..=40).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 40");
    }
    if let Some(response) = ensure_stock_exists(&symbol).await {
        return response;
    }
    let pool = database::get_connection();
    let limit = i64::from(limit);

    // 三個查詢都用 DISTINCT ON 讓同一期只留一列，排序時 Yahoo 優先；主鍵以
    // stock_symbol 開頭，單檔約百列，直接走主鍵索引。
    let income: Vec<IncomeRow> = match sqlx::query_as(
        r#"SELECT DISTINCT ON (fiscal_year, quarter) fiscal_year, quarter, source, revenue, gross_profit, selling_expenses, admin_expenses, rd_expenses, operating_expenses, operating_profit, non_operating_income, profit_before_tax, net_income, owner_parent_profit, revenue_per_share, operating_profit_per_share, profit_before_tax_per_share, eps, bps, updated_at FROM income_statement WHERE stock_symbol = $1 AND period_type = $2 ORDER BY fiscal_year DESC, quarter DESC, (source = 'yahoo') DESC, source LIMIT $3"#,
    )
    .bind(&symbol)
    .bind(db_period)
    .bind(limit)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };

    // 資產負債表只有單季列；年度模式取第 4 季季末。
    let balance: Vec<BalanceRow> = match sqlx::query_as(
        r#"SELECT DISTINCT ON (fiscal_year, quarter) fiscal_year, quarter, source, cash_and_equivalents, short_term_investments, accounts_receivable, inventory, other_current_assets, current_assets, equity_and_other_investments, property_plant_equipment, right_of_use_asset, non_current_assets, total_assets, long_term_investment, short_term_investment, other_assets, short_term_debt, short_term_bills_payable, accounts_payable, current_portion_of_long_term_liabilities, current_liabilities, long_term_liabilities, bonds_payable, other_liabilities, non_current_liabilities, total_liabilities, share_capital, retained_earnings, equity, book_value_per_share, updated_at FROM balance_sheet WHERE stock_symbol = $1 AND period_type = 'single' AND ($2::text = 'single' OR quarter = 'Q4') ORDER BY fiscal_year DESC, quarter DESC, (source = 'yahoo') DESC, source LIMIT $3"#,
    )
    .bind(&symbol)
    .bind(db_period)
    .bind(limit)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };

    let cash_flow: Vec<CashFlowRow> = match sqlx::query_as(
        r#"SELECT DISTINCT ON (fiscal_year, quarter) fiscal_year, quarter, source, depreciation, amortization, operating_cash_flow, investing_cash_flow, financing_cash_flow, free_cash_flow, net_cash_flow, updated_at FROM cash_flow_statement WHERE stock_symbol = $1 AND period_type = $2 ORDER BY fiscal_year DESC, quarter DESC, (source = 'yahoo') DESC, source LIMIT $3"#,
    )
    .bind(&symbol)
    .bind(db_period)
    .bind(limit)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => return database_error(error),
    };

    let data_as_of = income
        .first()
        .map(|row| period_label(row.fiscal_year, &row.quarter))
        .or_else(|| {
            cash_flow
                .first()
                .map(|row| period_label(row.fiscal_year, &row.quarter))
        })
        .or_else(|| {
            balance
                .first()
                .map(|row| period_label(row.fiscal_year, &row.quarter))
        });
    Json(FinancialReportResponse {
        income_statements: income
            .into_iter()
            .map(|row| row.into_dto(&symbol))
            .collect(),
        balance_sheets: balance
            .into_iter()
            .map(|row| row.into_dto(&symbol))
            .collect(),
        cash_flow_statements: cash_flow.into_iter().map(CashFlowRow::into_dto).collect(),
        stock_symbol: symbol,
        period_type: period_type.to_owned(),
        amount_unit: AMOUNT_UNIT,
        data_as_of,
    })
    .into_response()
}

/// `income_statement` 的一列。
#[derive(sqlx::FromRow)]
struct IncomeRow {
    fiscal_year: i32,
    quarter: String,
    source: String,
    revenue: Option<i64>,
    gross_profit: Option<i64>,
    selling_expenses: Option<i64>,
    admin_expenses: Option<i64>,
    rd_expenses: Option<i64>,
    operating_expenses: Option<i64>,
    operating_profit: Option<i64>,
    non_operating_income: Option<i64>,
    profit_before_tax: Option<i64>,
    net_income: Option<i64>,
    owner_parent_profit: Option<i64>,
    revenue_per_share: Option<Decimal>,
    operating_profit_per_share: Option<Decimal>,
    profit_before_tax_per_share: Option<Decimal>,
    eps: Option<Decimal>,
    bps: Option<Decimal>,
    updated_at: DateTime<Utc>,
}

impl IncomeRow {
    /// 轉成 API DTO；每股欄位的轉換失敗會帶股票代號記錄。
    fn into_dto(self, symbol: &str) -> IncomeStatementItem {
        let convert = |value, field| analytical_decimal_to_f64(value, symbol, field);
        IncomeStatementItem {
            fiscal_year: self.fiscal_year,
            quarter: self.quarter,
            source: self.source,
            revenue: self.revenue,
            gross_profit: self.gross_profit,
            selling_expenses: self.selling_expenses,
            admin_expenses: self.admin_expenses,
            rd_expenses: self.rd_expenses,
            operating_expenses: self.operating_expenses,
            operating_profit: self.operating_profit,
            non_operating_income: self.non_operating_income,
            profit_before_tax: self.profit_before_tax,
            net_income: self.net_income,
            owner_parent_profit: self.owner_parent_profit,
            revenue_per_share: convert(self.revenue_per_share, "revenue_per_share"),
            operating_profit_per_share: convert(
                self.operating_profit_per_share,
                "operating_profit_per_share",
            ),
            profit_before_tax_per_share: convert(
                self.profit_before_tax_per_share,
                "profit_before_tax_per_share",
            ),
            eps: convert(self.eps, "eps"),
            bps: convert(self.bps, "bps"),
            updated_at: self.updated_at.to_rfc3339(),
        }
    }
}

/// `balance_sheet` 的一列。
#[derive(sqlx::FromRow)]
struct BalanceRow {
    fiscal_year: i32,
    quarter: String,
    source: String,
    cash_and_equivalents: Option<i64>,
    short_term_investments: Option<i64>,
    accounts_receivable: Option<i64>,
    inventory: Option<i64>,
    other_current_assets: Option<i64>,
    current_assets: Option<i64>,
    equity_and_other_investments: Option<i64>,
    property_plant_equipment: Option<i64>,
    right_of_use_asset: Option<i64>,
    non_current_assets: Option<i64>,
    total_assets: Option<i64>,
    long_term_investment: Option<i64>,
    short_term_investment: Option<i64>,
    other_assets: Option<i64>,
    short_term_debt: Option<i64>,
    short_term_bills_payable: Option<i64>,
    accounts_payable: Option<i64>,
    current_portion_of_long_term_liabilities: Option<i64>,
    current_liabilities: Option<i64>,
    long_term_liabilities: Option<i64>,
    bonds_payable: Option<i64>,
    other_liabilities: Option<i64>,
    non_current_liabilities: Option<i64>,
    total_liabilities: Option<i64>,
    share_capital: Option<i64>,
    retained_earnings: Option<i64>,
    equity: Option<i64>,
    book_value_per_share: Option<Decimal>,
    updated_at: DateTime<Utc>,
}

impl BalanceRow {
    /// 轉成 API DTO；每股淨值的轉換失敗會帶股票代號記錄。
    fn into_dto(self, symbol: &str) -> BalanceSheetItem {
        BalanceSheetItem {
            fiscal_year: self.fiscal_year,
            quarter: self.quarter,
            source: self.source,
            cash_and_equivalents: self.cash_and_equivalents,
            short_term_investments: self.short_term_investments,
            accounts_receivable: self.accounts_receivable,
            inventory: self.inventory,
            other_current_assets: self.other_current_assets,
            current_assets: self.current_assets,
            equity_and_other_investments: self.equity_and_other_investments,
            property_plant_equipment: self.property_plant_equipment,
            right_of_use_asset: self.right_of_use_asset,
            non_current_assets: self.non_current_assets,
            total_assets: self.total_assets,
            long_term_investment: self.long_term_investment,
            short_term_investment: self.short_term_investment,
            other_assets: self.other_assets,
            short_term_debt: self.short_term_debt,
            short_term_bills_payable: self.short_term_bills_payable,
            accounts_payable: self.accounts_payable,
            current_portion_of_long_term_liabilities: self.current_portion_of_long_term_liabilities,
            current_liabilities: self.current_liabilities,
            long_term_liabilities: self.long_term_liabilities,
            bonds_payable: self.bonds_payable,
            other_liabilities: self.other_liabilities,
            non_current_liabilities: self.non_current_liabilities,
            total_liabilities: self.total_liabilities,
            share_capital: self.share_capital,
            retained_earnings: self.retained_earnings,
            equity: self.equity,
            book_value_per_share: analytical_decimal_to_f64(
                self.book_value_per_share,
                symbol,
                "book_value_per_share",
            ),
            updated_at: self.updated_at.to_rfc3339(),
        }
    }
}

/// `cash_flow_statement` 的一列。
#[derive(sqlx::FromRow)]
struct CashFlowRow {
    fiscal_year: i32,
    quarter: String,
    source: String,
    depreciation: Option<i64>,
    amortization: Option<i64>,
    operating_cash_flow: Option<i64>,
    investing_cash_flow: Option<i64>,
    financing_cash_flow: Option<i64>,
    free_cash_flow: Option<i64>,
    net_cash_flow: Option<i64>,
    updated_at: DateTime<Utc>,
}

impl CashFlowRow {
    /// 轉成 API DTO；現金流量表全是整數金額，不需數值轉換。
    fn into_dto(self) -> CashFlowStatementItem {
        CashFlowStatementItem {
            fiscal_year: self.fiscal_year,
            quarter: self.quarter,
            source: self.source,
            depreciation: self.depreciation,
            amortization: self.amortization,
            operating_cash_flow: self.operating_cash_flow,
            investing_cash_flow: self.investing_cash_flow,
            financing_cash_flow: self.financing_cash_flow,
            free_cash_flow: self.free_cash_flow,
            net_cash_flow: self.net_cash_flow,
            updated_at: self.updated_at.to_rfc3339(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只接受 quarterly／annual；`all` 對三張分開的報表沒有意義，視為參數錯誤。
    #[test]
    fn period_type_maps_to_table_values() {
        assert_eq!(db_period_type("quarterly"), Some("single"));
        assert_eq!(db_period_type("annual"), Some("annual"));
        for invalid in ["all", "single", "Quarterly", ""] {
            assert_eq!(db_period_type(invalid), None, "{invalid:?} 應不合法");
        }
    }

    /// 期間標記沿用資料表的季別字串。
    #[test]
    fn period_label_joins_year_and_quarter() {
        assert_eq!(period_label(2026, "Q2"), "2026-Q2");
        assert_eq!(period_label(2025, "A"), "2025-A");
    }
}
