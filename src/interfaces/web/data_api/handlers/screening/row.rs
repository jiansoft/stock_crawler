//! 條件選股的資料庫列型別與 API 契約轉換。

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::super::{analytical_decimal_to_f64, format_month};
use crate::interfaces::web::data_api::dto::ScreenedStock;

/// 對應條件選股正規化後的資料庫列。
///
/// 指標與分類使用 `Option` 表達「來源不存在或已過期」；四個來源期間另行保留，
/// 因此呼叫端仍能辨識是完全沒有資料，或只是超過新鮮度上限。
#[derive(sqlx::FromRow)]
pub(super) struct ScreenedStockRow {
    /// 股票代號。
    stock_symbol: String,
    /// 股票名稱。
    name: String,
    /// 上市或上櫃市場 id。
    market_id: i32,
    /// 產業分類 id。
    industry_id: i32,
    /// 新鮮營收年增率。
    revenue_yoy_percent: Option<Decimal>,
    /// 新鮮的連續營收年增月數。
    revenue_yoy_streak: Option<i16>,
    /// 新鮮的近 3 個月累計營收年增率。
    revenue_yoy_3m_percent: Option<Decimal>,
    /// 新鮮的最新月營收（千元）。
    monthly_revenue: Option<Decimal>,
    /// 新鮮季度 EPS。
    earnings_per_share: Option<Decimal>,
    /// 新鮮季度 ROE。
    return_on_equity: Option<Decimal>,
    /// 新鮮殖利率。
    dividend_yield_percent: Option<Decimal>,
    /// 新鮮估值分類。
    valuation_band: Option<String>,
    /// 新鮮估值百分比。
    valuation_percentage: Option<Decimal>,
    /// 最新營收月份的資料庫 `YYYYMM` 編碼。
    revenue_month: Option<i64>,
    /// 最新季度財報年度。
    financial_year: Option<i64>,
    /// 最新季度財報季別。
    financial_quarter: Option<String>,
    /// 最新估值日期。
    valuation_date: Option<NaiveDate>,
    /// 最新殖利率日期。
    yield_date: Option<NaiveDate>,
    /// 新鮮的 F 分數。
    f_score: Option<i16>,
    /// 新鮮的 F 分數可判定指標數。
    f_score_evaluated: Option<i16>,
    /// F 分數評分年度。
    f_score_year: Option<i32>,
    /// 新鮮的單季毛利率。
    gross_margin_percent: Option<Decimal>,
    /// 新鮮的季末負債比。
    debt_ratio_percent: Option<Decimal>,
    /// 新鮮的近四季自由現金流。
    free_cash_flow_ttm: Option<i64>,
    /// 季報指標期間年度。
    health_quarter_year: Option<i32>,
    /// 季報指標期間季別。
    health_quarter: Option<i16>,
}

impl From<ScreenedStockRow> for ScreenedStock {
    /// 將 SQL NULL 與來源期間轉成固定 API 契約。
    fn from(row: ScreenedStockRow) -> Self {
        let symbol = row.stock_symbol.clone();
        // 年度與季度必須同時存在才組成有效期間；LATERAL 財報列本身保證兩者
        // 同進同出，這個 zip 仍避免異常資料被輸出成不完整字串。
        let financial_period = row
            .financial_year
            .zip(row.financial_quarter)
            .map(|(year, quarter)| format!("{year}-{quarter}"));
        let health_period = row
            .health_quarter_year
            .zip(row.health_quarter)
            .map(|(year, quarter)| format!("{year}-Q{quarter}"));
        ScreenedStock {
            stock_symbol: row.stock_symbol,
            name: row.name,
            market_id: row.market_id,
            industry_id: row.industry_id,
            revenue_yoy_percent: analytical_decimal_to_f64(
                row.revenue_yoy_percent,
                &symbol,
                "revenue_yoy_percent",
            ),
            revenue_yoy_streak: row
                .revenue_yoy_streak
                .and_then(|months| u8::try_from(months).ok()),
            revenue_yoy_3m_percent: analytical_decimal_to_f64(
                row.revenue_yoy_3m_percent,
                &symbol,
                "revenue_yoy_3m_percent",
            ),
            monthly_revenue: analytical_decimal_to_f64(
                row.monthly_revenue,
                &symbol,
                "monthly_revenue",
            ),
            earnings_per_share: analytical_decimal_to_f64(
                row.earnings_per_share,
                &symbol,
                "earnings_per_share",
            ),
            return_on_equity: analytical_decimal_to_f64(
                row.return_on_equity,
                &symbol,
                "return_on_equity",
            ),
            dividend_yield_percent: analytical_decimal_to_f64(
                row.dividend_yield_percent,
                &symbol,
                "dividend_yield_percent",
            ),
            valuation_band: row.valuation_band,
            valuation_percentage: analytical_decimal_to_f64(
                row.valuation_percentage,
                &symbol,
                "valuation_percentage",
            ),
            revenue_month: row.revenue_month.map(format_month),
            financial_period,
            valuation_date: row.valuation_date.map(|date| date.to_string()),
            yield_date: row.yield_date.map(|date| date.to_string()),
            f_score: row.f_score.and_then(|score| u8::try_from(score).ok()),
            f_score_evaluated: row
                .f_score_evaluated
                .and_then(|evaluated| u8::try_from(evaluated).ok()),
            f_score_year: row.f_score_year,
            gross_margin_percent: analytical_decimal_to_f64(
                row.gross_margin_percent,
                &symbol,
                "gross_margin_percent",
            ),
            debt_ratio_percent: analytical_decimal_to_f64(
                row.debt_ratio_percent,
                &symbol,
                "debt_ratio_percent",
            ),
            free_cash_flow_ttm: row.free_cash_flow_ttm,
            health_period,
        }
    }
}
