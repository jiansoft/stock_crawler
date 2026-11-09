//! 選股結果的排序：`sort_by`／`sort_order` 白名單對應到固定的 `ORDER BY` 子句。

/// 將排序 enum 映射成四十六個固定 SQL 分支。
///
/// 指標欄位一律加 `NULLS LAST`，否則 PostgreSQL 的降冪預設會把過期或缺值的
/// NULL 排在最前面；同值再以股票代號升冪，讓分頁外的重複查詢仍穩定。
pub(super) fn screen_order_by(
    sort_by: &str,
    sort_order: &str,
) -> Result<&'static str, &'static str> {
    match (sort_by, sort_order) {
        ("stock_symbol", "asc") => Ok("ORDER BY stock_symbol ASC"),
        ("stock_symbol", "desc") => Ok("ORDER BY stock_symbol DESC"),
        ("pe_ratio", "asc") => Ok("ORDER BY pe_ratio ASC NULLS LAST, stock_symbol ASC"),
        ("pe_ratio", "desc") => Ok("ORDER BY pe_ratio DESC NULLS LAST, stock_symbol ASC"),
        ("pb_ratio", "asc") => Ok("ORDER BY pb_ratio ASC NULLS LAST, stock_symbol ASC"),
        ("pb_ratio", "desc") => Ok("ORDER BY pb_ratio DESC NULLS LAST, stock_symbol ASC"),
        ("revenue_yoy", "asc") => {
            Ok("ORDER BY revenue_yoy_percent ASC NULLS LAST, stock_symbol ASC")
        }
        ("revenue_yoy", "desc") => {
            Ok("ORDER BY revenue_yoy_percent DESC NULLS LAST, stock_symbol ASC")
        }
        ("revenue_yoy_streak", "asc") => {
            Ok("ORDER BY revenue_yoy_streak ASC NULLS LAST, stock_symbol ASC")
        }
        ("revenue_yoy_streak", "desc") => {
            Ok("ORDER BY revenue_yoy_streak DESC NULLS LAST, stock_symbol ASC")
        }
        ("revenue_yoy_3m", "asc") => {
            Ok("ORDER BY revenue_yoy_3m_percent ASC NULLS LAST, stock_symbol ASC")
        }
        ("revenue_yoy_3m", "desc") => {
            Ok("ORDER BY revenue_yoy_3m_percent DESC NULLS LAST, stock_symbol ASC")
        }
        ("monthly_revenue", "asc") => {
            Ok("ORDER BY monthly_revenue ASC NULLS LAST, stock_symbol ASC")
        }
        ("monthly_revenue", "desc") => {
            Ok("ORDER BY monthly_revenue DESC NULLS LAST, stock_symbol ASC")
        }
        ("eps", "asc") => Ok("ORDER BY earnings_per_share ASC NULLS LAST, stock_symbol ASC"),
        ("eps", "desc") => Ok("ORDER BY earnings_per_share DESC NULLS LAST, stock_symbol ASC"),
        ("roe", "asc") => Ok("ORDER BY return_on_equity ASC NULLS LAST, stock_symbol ASC"),
        ("roe", "desc") => Ok("ORDER BY return_on_equity DESC NULLS LAST, stock_symbol ASC"),
        ("dividend_yield", "asc") => {
            Ok("ORDER BY dividend_yield_percent ASC NULLS LAST, stock_symbol ASC")
        }
        ("dividend_yield", "desc") => {
            Ok("ORDER BY dividend_yield_percent DESC NULLS LAST, stock_symbol ASC")
        }
        ("valuation_percentage", "asc") => {
            Ok("ORDER BY valuation_percentage ASC NULLS LAST, stock_symbol ASC")
        }
        ("valuation_percentage", "desc") => {
            Ok("ORDER BY valuation_percentage DESC NULLS LAST, stock_symbol ASC")
        }
        ("f_score", "asc") => Ok("ORDER BY f_score ASC NULLS LAST, stock_symbol ASC"),
        ("f_score", "desc") => Ok("ORDER BY f_score DESC NULLS LAST, stock_symbol ASC"),
        ("gross_margin", "asc") => {
            Ok("ORDER BY gross_margin_percent ASC NULLS LAST, stock_symbol ASC")
        }
        ("gross_margin", "desc") => {
            Ok("ORDER BY gross_margin_percent DESC NULLS LAST, stock_symbol ASC")
        }
        ("debt_ratio", "asc") => Ok("ORDER BY debt_ratio_percent ASC NULLS LAST, stock_symbol ASC"),
        ("debt_ratio", "desc") => {
            Ok("ORDER BY debt_ratio_percent DESC NULLS LAST, stock_symbol ASC")
        }
        ("operating_margin", "asc") => {
            Ok("ORDER BY operating_margin_percent ASC NULLS LAST, stock_symbol ASC")
        }
        ("operating_margin", "desc") => {
            Ok("ORDER BY operating_margin_percent DESC NULLS LAST, stock_symbol ASC")
        }
        ("rs_score", "asc") => Ok("ORDER BY rs_score ASC NULLS LAST, stock_symbol ASC"),
        ("rs_score", "desc") => Ok("ORDER BY rs_score DESC NULLS LAST, stock_symbol ASC"),
        ("eps_yoy", "asc") => Ok("ORDER BY eps_yoy_percent ASC NULLS LAST, stock_symbol ASC"),
        ("eps_yoy", "desc") => Ok("ORDER BY eps_yoy_percent DESC NULLS LAST, stock_symbol ASC"),
        ("annual_eps_growth", "asc") => {
            Ok("ORDER BY annual_eps_growth_percent ASC NULLS LAST, stock_symbol ASC")
        }
        ("annual_eps_growth", "desc") => {
            Ok("ORDER BY annual_eps_growth_percent DESC NULLS LAST, stock_symbol ASC")
        }
        ("magic_formula", "asc") => {
            Ok("ORDER BY magic_formula_rank ASC NULLS LAST, stock_symbol ASC")
        }
        ("magic_formula", "desc") => {
            Ok("ORDER BY magic_formula_rank DESC NULLS LAST, stock_symbol ASC")
        }
        ("peg", "asc") => Ok("ORDER BY peg_ratio ASC NULLS LAST, stock_symbol ASC"),
        ("peg", "desc") => Ok("ORDER BY peg_ratio DESC NULLS LAST, stock_symbol ASC"),
        ("volatility", "asc") => Ok("ORDER BY volatility_percent ASC NULLS LAST, stock_symbol ASC"),
        ("volatility", "desc") => {
            Ok("ORDER BY volatility_percent DESC NULLS LAST, stock_symbol ASC")
        }
        ("beta", "asc") => Ok("ORDER BY beta ASC NULLS LAST, stock_symbol ASC"),
        ("beta", "desc") => Ok("ORDER BY beta DESC NULLS LAST, stock_symbol ASC"),
        ("bias240", "asc") => Ok("ORDER BY bias240_percent ASC NULLS LAST, stock_symbol ASC"),
        ("bias240", "desc") => Ok("ORDER BY bias240_percent DESC NULLS LAST, stock_symbol ASC"),
        (_, "asc" | "desc") => Err("sort_by 不在允許範圍"),
        _ => Err("sort_order 必須為 asc 或 desc"),
    }
}
