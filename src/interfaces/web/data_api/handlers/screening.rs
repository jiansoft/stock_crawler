//! `/stocks/screen` 條件選股 handler、其參數化 SQL、參數驗證與排序
//! 白名單，以及對應的資料庫列型別。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
#[cfg(test)]
use chrono::Datelike;
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;
use std::str::FromStr;

use super::{
    analytical_decimal_to_f64, database_error, error_response, format_month, market_id_for_stocks,
};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    ErrorBody, ScreenedStock, StockScreeningParams, StockScreeningResponse,
};

/// 以固定條件篩選上市櫃股票（§4.7）。
///
/// 每張分析表都先依股票代號取得該股票自己的最新一期，避免月營收分批公布
/// 時以全市場最大日期誤刪尚未公布的股票。接著以台北本地查詢日套用各來源
/// 的新鮮度上限；過期指標轉成 `null`，來源日期則保留供呼叫端稽核。
///
/// # Errors
///
/// 參數不在固定 enum／數值範圍、沒有任何有效篩選條件時回 422；資料庫查詢
/// 失敗時只記錄 server log，對外回不含 SQL 與連線資訊的安全 500 訊息。
#[utoipa::path(get, path = "/api/v1/stocks/screen", tag = "data-api", params(StockScreeningParams), responses((status = 200, body = StockScreeningResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn screen_stocks(Query(params): Query<StockScreeningParams>) -> Response {
    let validated = match validate_screening_params(&params) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };

    // 查詢日由 handler 明確綁定，SQL 不直接讀 CURRENT_DATE；純函式測試可傳入
    // 任意日期驗證月份、季度與 31 天邊界，production 則採伺服器台北本地日。
    let query_date = Local::now().date_naive();
    // ORDER BY 字串只可能來自下方十二個 &'static str 分支，絕不插入呼叫端
    // 原始文字；其他條件仍全部使用 bind parameter。
    let sql = format!("{SCREEN_STOCKS_SQL}\n{}\nLIMIT $9", validated.order_by);
    // SQLx 0.9 要求動態字串顯式稽核；此處唯一動態部分已由
    // `screen_order_by` 限制為十二個靜態常數，因此可安全標記。
    let rows: Result<Vec<ScreenedStockRow>, _> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(validated.market_id)
        .bind(params.industry_id)
        .bind(query_date)
        .bind(validated.min_revenue_yoy_percent)
        .bind(validated.min_eps)
        .bind(validated.min_roe_percent)
        .bind(validated.min_dividend_yield_percent)
        .bind(params.valuation_band.as_deref())
        .bind(i64::from(validated.limit))
        .fetch_all(database::get_connection())
        .await;

    match rows {
        Ok(rows) => Json(StockScreeningResponse {
            // 每股四種資料可能來自不同日期，因此不可偽造單一 data_as_of。
            data_as_of: None,
            stocks: rows.into_iter().map(Into::into).collect(),
        })
        .into_response(),
        Err(error) => database_error(error),
    }
}

/// 條件選股的參數化 SQL 主體。
///
/// `LATERAL ... LIMIT 1` 會使用各表以股票代號為前導欄位的索引，找出「每股
/// 各自最新」資料。`normalized` 僅將過期指標轉成 SQL NULL；最外層條件利用
/// NULL 比較不成立的語意，自然排除以該過期指標篩選的股票。
const SCREEN_STOCKS_SQL: &str = r#"
WITH latest AS (
    SELECT
        s.stock_symbol,
        s."Name" AS name,
        s.stock_exchange_market_id AS market_id,
        s.stock_industry_id AS industry_id,
        r."Date" AS revenue_raw_month,
        r."ComparedWithLastYearSameMonth" AS revenue_raw_yoy,
        f.year AS financial_year,
        f.quarter AS financial_quarter,
        f.earnings_per_share AS eps_raw,
        f.return_on_equity AS roe_raw,
        e.date AS valuation_raw_date,
        e.percentage AS valuation_raw_percentage,
        e.closing_price,
        e.cheap,
        e.fair,
        e.expensive,
        y.date AS yield_raw_date,
        y.yield AS yield_raw
    FROM stocks s
    LEFT JOIN LATERAL (
        SELECT "Date", "ComparedWithLastYearSameMonth"
        FROM "Revenue"
        WHERE "SecurityCode" = s.stock_symbol
        ORDER BY "Date" DESC
        LIMIT 1
    ) r ON TRUE
    LEFT JOIN LATERAL (
        SELECT year, quarter, earnings_per_share, return_on_equity
        FROM financial_statement
        WHERE security_code = s.stock_symbol
          AND quarter IN ('Q1', 'Q2', 'Q3', 'Q4')
        ORDER BY year DESC,
            CASE quarter
                WHEN 'Q4' THEN 4 WHEN 'Q3' THEN 3
                WHEN 'Q2' THEN 2 WHEN 'Q1' THEN 1
            END DESC
        LIMIT 1
    ) f ON TRUE
    LEFT JOIN LATERAL (
        SELECT date, percentage, closing_price, cheap, fair, expensive
        FROM estimate
        WHERE security_code = s.stock_symbol
        ORDER BY date DESC
        LIMIT 1
    ) e ON TRUE
    LEFT JOIN LATERAL (
        SELECT date, yield
        FROM yield_rank
        WHERE security_code = s.stock_symbol
        ORDER BY date DESC
        LIMIT 1
    ) y ON TRUE
    WHERE (($1::int = 0 AND s.stock_exchange_market_id IN (2, 4))
        OR s.stock_exchange_market_id = $1)
      AND ($2::int IS NULL OR s.stock_industry_id = $2)
), normalized AS (
    SELECT
        latest.*,
        CASE WHEN revenue_raw_month IS NOT NULL
          AND ((EXTRACT(YEAR FROM $3::date)::int * 12
                + EXTRACT(MONTH FROM $3::date)::int)
              - ((revenue_raw_month / 100)::int * 12
                + (revenue_raw_month % 100)::int)) BETWEEN 0 AND 3
          THEN revenue_raw_yoy END AS revenue_yoy_percent,
        CASE WHEN financial_year IS NOT NULL
          AND ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (financial_year::int * 4
                + SUBSTRING(financial_quarter FROM 2)::int)) BETWEEN 0 AND 2
          THEN eps_raw END AS earnings_per_share,
        CASE WHEN financial_year IS NOT NULL
          AND ((EXTRACT(YEAR FROM $3::date)::int * 4
                + EXTRACT(QUARTER FROM $3::date)::int)
              - (financial_year::int * 4
                + SUBSTRING(financial_quarter FROM 2)::int)) BETWEEN 0 AND 2
          THEN roe_raw END AS return_on_equity,
        CASE WHEN valuation_raw_date BETWEEN $3::date - 30 AND $3::date
          THEN valuation_raw_percentage END AS valuation_percentage,
        CASE WHEN valuation_raw_date BETWEEN $3::date - 30 AND $3::date THEN
            CASE
                WHEN closing_price <= cheap THEN 'undervalued'
                WHEN closing_price <= fair THEN 'fair_valued'
                WHEN closing_price <= expensive THEN 'overvalued'
                ELSE 'highly_overvalued'
            END
        END AS valuation_band,
        CASE WHEN yield_raw_date BETWEEN $3::date - 30 AND $3::date
          THEN yield_raw END AS dividend_yield_percent
    FROM latest
)
SELECT
    stock_symbol, name, market_id, industry_id,
    revenue_yoy_percent, earnings_per_share, return_on_equity,
    dividend_yield_percent, valuation_band, valuation_percentage,
    revenue_raw_month AS revenue_month,
    financial_year, financial_quarter,
    valuation_raw_date AS valuation_date,
    yield_raw_date AS yield_date
FROM normalized
WHERE ($4::numeric IS NULL OR revenue_yoy_percent >= $4)
  AND ($5::numeric IS NULL OR earnings_per_share >= $5)
  AND ($6::numeric IS NULL OR return_on_equity >= $6)
  AND ($7::numeric IS NULL OR dividend_yield_percent >= $7)
  AND ($8::text IS NULL OR valuation_band = $8)
"#;

/// 驗證後可安全綁定條件選股 SQL 的內部參數。
///
/// 數值門檻預先轉為 `Decimal`，使 PostgreSQL 能以原生 `NUMERIC` 比較而不會
/// 因浮點參數產生隱含型別轉換；排序則只保留程式內建的靜態 SQL 片段。
struct ValidatedScreeningParams {
    /// 股票表市場 id；0 只代表 SQL 內固定展開的上市加上櫃。
    market_id: i32,
    /// 最低營收年增率。
    min_revenue_yoy_percent: Option<Decimal>,
    /// 最低每股盈餘。
    min_eps: Option<Decimal>,
    /// 最低股東權益報酬率。
    min_roe_percent: Option<Decimal>,
    /// 最低殖利率。
    min_dividend_yield_percent: Option<Decimal>,
    /// 十二個固定排序分支之一。
    order_by: &'static str,
    /// 查詢筆數上限。
    limit: u8,
}

/// 驗證條件選股參數並轉換成資料庫可直接綁定的型別。
///
/// `market=twse/tpex` 本身就是有效篩選；`market=all` 不縮小上市櫃集合，
/// 因此必須再提供產業、估值或至少一個數值門檻，避免 endpoint 被當成無條件
/// 的全市場匯出介面。
fn validate_screening_params(
    params: &StockScreeningParams,
) -> Result<ValidatedScreeningParams, &'static str> {
    let market = params.market.as_deref().unwrap_or("all");
    let market_id = market_id_for_stocks(market).ok_or("market 必須為 all、twse 或 tpex")?;
    if params.industry_id.is_some_and(|value| value <= 0) {
        return Err("industry_id 必須為正整數");
    }
    if params.valuation_band.as_deref().is_some_and(|value| {
        ![
            "undervalued",
            "fair_valued",
            "overvalued",
            "highly_overvalued",
        ]
        .contains(&value)
    }) {
        return Err("valuation_band 不在允許範圍");
    }
    let min_revenue_yoy_percent = decimal_in_range(
        params.min_revenue_yoy_percent,
        -100.0,
        10_000.0,
        "min_revenue_yoy_percent 必須介於 -100 至 10000",
    )?;
    let min_eps = decimal_in_range(
        params.min_eps,
        -10_000.0,
        10_000.0,
        "min_eps 必須介於 -10000 至 10000",
    )?;
    let min_roe_percent = decimal_in_range(
        params.min_roe_percent,
        -10_000.0,
        10_000.0,
        "min_roe_percent 必須介於 -10000 至 10000",
    )?;
    let min_dividend_yield_percent = decimal_in_range(
        params.min_dividend_yield_percent,
        0.0,
        1_000.0,
        "min_dividend_yield_percent 必須介於 0 至 1000",
    )?;
    let order_by = screen_order_by(
        params.sort_by.as_deref().unwrap_or("stock_symbol"),
        params.sort_order.as_deref().unwrap_or("asc"),
    )?;
    let limit = params.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return Err("limit 必須介於 1 至 50");
    }
    let has_filter = market != "all"
        || params.industry_id.is_some()
        || params.valuation_band.is_some()
        || params.min_revenue_yoy_percent.is_some()
        || params.min_eps.is_some()
        || params.min_roe_percent.is_some()
        || params.min_dividend_yield_percent.is_some();
    if !has_filter {
        return Err("至少需要一個篩選條件");
    }
    Ok(ValidatedScreeningParams {
        market_id,
        min_revenue_yoy_percent,
        min_eps,
        min_roe_percent,
        min_dividend_yield_percent,
        order_by,
        limit,
    })
}

/// 將有限範圍的 HTTP 浮點數轉成 PostgreSQL `NUMERIC` 綁定值。
///
/// JSON／query parser 可能接受 `NaN` 或無限大；先要求有限值，接著透過十進位
/// 字串建構 Decimal，確保 SQL 比較值與使用者輸入的十進位表示一致。
fn decimal_in_range(
    value: Option<f64>,
    minimum: f64,
    maximum: f64,
    error: &'static str,
) -> Result<Option<Decimal>, &'static str> {
    value
        .map(|number| {
            if !number.is_finite() || !(minimum..=maximum).contains(&number) {
                return Err(error);
            }
            Decimal::from_str(&number.to_string()).map_err(|_| error)
        })
        .transpose()
}

/// 將排序 enum 映射成十二個固定 SQL 分支。
///
/// 指標欄位一律加 `NULLS LAST`，否則 PostgreSQL 的降冪預設會把過期或缺值的
/// NULL 排在最前面；同值再以股票代號升冪，讓分頁外的重複查詢仍穩定。
fn screen_order_by(sort_by: &str, sort_order: &str) -> Result<&'static str, &'static str> {
    match (sort_by, sort_order) {
        ("stock_symbol", "asc") => Ok("ORDER BY stock_symbol ASC"),
        ("stock_symbol", "desc") => Ok("ORDER BY stock_symbol DESC"),
        ("revenue_yoy", "asc") => {
            Ok("ORDER BY revenue_yoy_percent ASC NULLS LAST, stock_symbol ASC")
        }
        ("revenue_yoy", "desc") => {
            Ok("ORDER BY revenue_yoy_percent DESC NULLS LAST, stock_symbol ASC")
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
        (_, "asc" | "desc") => Err("sort_by 不在允許範圍"),
        _ => Err("sort_order 必須為 asc 或 desc"),
    }
}

/// 測試 SQL 月份新鮮度公式的純 Rust 對照實作。
#[cfg(test)]
fn revenue_month_is_fresh(query_date: NaiveDate, revenue_month: i64) -> bool {
    let query_ordinal = i64::from(query_date.year()) * 12 + i64::from(query_date.month());
    let revenue_ordinal = (revenue_month / 100) * 12 + revenue_month % 100;
    (0..=3).contains(&(query_ordinal - revenue_ordinal))
}

/// 測試 SQL 季度新鮮度公式的純 Rust 對照實作。
#[cfg(test)]
fn financial_period_is_fresh(query_date: NaiveDate, year: i64, quarter: &str) -> bool {
    let Some(quarter) = quarter
        .strip_prefix('Q')
        .and_then(|raw| raw.parse::<i64>().ok())
    else {
        return false;
    };
    let query_ordinal = i64::from(query_date.year()) * 4 + i64::from(query_date.month0() / 3 + 1);
    let financial_ordinal = year * 4 + quarter;
    (0..=2).contains(&(query_ordinal - financial_ordinal))
}

/// 測試 SQL 31 個日曆日視窗的純 Rust 對照實作。
#[cfg(test)]
fn analytical_date_is_fresh(query_date: NaiveDate, source_date: NaiveDate) -> bool {
    source_date <= query_date && source_date >= query_date - chrono::Days::new(30)
}

/// 對應條件選股正規化後的資料庫列。
///
/// 指標與分類使用 `Option` 表達「來源不存在或已過期」；四個來源期間另行保留，
/// 因此呼叫端仍能辨識是完全沒有資料，或只是超過新鮮度上限。
#[derive(sqlx::FromRow)]
struct ScreenedStockRow {
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
        }
    }
}

#[cfg(test)]
mod tests {
    //! 條件選股參數驗證、排序白名單與新鮮度公式的 deterministic tests，
    //! 以及以真實 PostgreSQL 驗證執行計畫的 P0-4 整合測試。

    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    use super::{
        SCREEN_STOCKS_SQL, analytical_date_is_fresh, financial_period_is_fresh,
        revenue_month_is_fresh, screen_order_by, validate_screening_params,
    };
    use crate::infra::database;
    use crate::interfaces::web::data_api::dto::StockScreeningParams;

    /// 建立只覆寫測試重點欄位的 screen 預設參數。
    fn screening_params() -> StockScreeningParams {
        StockScreeningParams {
            market: None,
            industry_id: None,
            valuation_band: None,
            min_revenue_yoy_percent: None,
            min_eps: None,
            min_roe_percent: None,
            min_dividend_yield_percent: None,
            sort_by: None,
            sort_order: None,
            limit: None,
        }
    }

    /// `all` 不縮小市場集合，不能單獨通過；上市／上櫃或任何一個實質條件
    /// 都可成立，固定預設排序與 limit 也應在驗證後補齊。
    #[test]
    fn screening_requires_a_meaningful_filter() {
        assert_eq!(
            validate_screening_params(&screening_params()).err(),
            Some("至少需要一個篩選條件")
        );
        let mut explicit_all = screening_params();
        explicit_all.market = Some("all".to_owned());
        assert!(validate_screening_params(&explicit_all).is_err());

        let mut twse = screening_params();
        twse.market = Some("twse".to_owned());
        let validated = validate_screening_params(&twse).expect("twse 本身是有效條件");
        assert_eq!(validated.market_id, 2);
        assert_eq!(validated.limit, 20);
        assert_eq!(validated.order_by, "ORDER BY stock_symbol ASC");

        let mut industry = screening_params();
        industry.industry_id = Some(24);
        assert!(validate_screening_params(&industry).is_ok());
        let mut minimum = screening_params();
        minimum.min_eps = Some(0.0);
        assert!(validate_screening_params(&minimum).is_ok());
    }

    /// 六個排序欄位乘上兩種方向必須完整映射，指標降冪也要明確
    /// `NULLS LAST`；不在白名單的文字必須在接觸 SQL 前遭拒。
    #[test]
    fn screening_sort_has_twelve_static_branches() {
        let fields = [
            "stock_symbol",
            "revenue_yoy",
            "eps",
            "roe",
            "dividend_yield",
            "valuation_percentage",
        ];
        for field in fields {
            for order in ["asc", "desc"] {
                let sql = screen_order_by(field, order).expect("白名單組合應存在");
                assert!(sql.starts_with("ORDER BY "));
                if field != "stock_symbol" {
                    assert!(sql.contains("NULLS LAST"));
                    assert!(sql.ends_with("stock_symbol ASC"));
                }
            }
        }
        assert!(screen_order_by("closing_price; DROP TABLE stocks", "asc").is_err());
        assert!(screen_order_by("eps", "sideways").is_err());
    }

    /// 數值範圍包含契約兩端，超界、NaN 與無限大都必須回 422；這可避免
    /// 非有限浮點數進入 Decimal 轉換或 PostgreSQL 比較。
    #[test]
    fn screening_numeric_boundaries_are_deterministic() {
        for value in [-100.0, 10_000.0] {
            let mut params = screening_params();
            params.min_revenue_yoy_percent = Some(value);
            assert!(validate_screening_params(&params).is_ok());
        }
        for value in [-100.01, 10_000.01, f64::NAN, f64::INFINITY] {
            let mut params = screening_params();
            params.min_revenue_yoy_percent = Some(value);
            assert!(validate_screening_params(&params).is_err());
        }
        let mut yield_at_zero = screening_params();
        yield_at_zero.min_dividend_yield_percent = Some(0.0);
        assert!(validate_screening_params(&yield_at_zero).is_ok());
        let mut negative_yield = screening_params();
        negative_yield.min_dividend_yield_percent = Some(-0.01);
        assert!(validate_screening_params(&negative_yield).is_err());
    }

    /// 新鮮度邊界使用月份／季度序號處理跨年，並以「差三月／兩季／三十日」
    /// 為仍有效的最後一天；未來日期一律不視為新鮮。
    #[test]
    fn screening_freshness_boundaries_cross_years() {
        let query_date = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        assert!(revenue_month_is_fresh(query_date, 202510));
        assert!(!revenue_month_is_fresh(query_date, 202509));
        assert!(!revenue_month_is_fresh(query_date, 202602));

        assert!(financial_period_is_fresh(query_date, 2025, "Q3"));
        assert!(!financial_period_is_fresh(query_date, 2025, "Q2"));
        assert!(!financial_period_is_fresh(query_date, 2026, "Q2"));
        assert!(!financial_period_is_fresh(query_date, 2025, ""));

        let thirty_days_ago = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let thirty_one_days_ago = NaiveDate::from_ymd_opt(2025, 12, 31).unwrap();
        assert!(analytical_date_is_fresh(query_date, thirty_days_ago));
        assert!(!analytical_date_is_fresh(query_date, thirty_one_days_ago));
        assert!(!analytical_date_is_fresh(
            query_date,
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()
        ));
    }

    /// P0-4：以實際 PostgreSQL 對 Phase 2/3 核心查詢執行 EXPLAIN ANALYZE。
    ///
    /// 測試輸出文字 plan 供人工記錄 latency 與 scan 類型，並固定條件選股
    /// 四個「每股最新一期」子查詢使用既有索引；沒有資料庫時明確跳過。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
    )]
    async fn phase2_phase3_query_plans_use_existing_indexes() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 P0-4 EXPLAIN：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        // EXPLAIN 只需要型別正確的查詢參數，不需要依賴資料庫預先匯入股票。
        // 使用固定代號可讓全新 CI 資料庫也能驗證執行計畫，且不會寫入測試資料。
        let symbol = "2330";

        // SQL 與 production 使用相同 WHERE、JOIN、排序及 LIMIT；EXPLAIN 不寫資料。
        let valuation: Vec<String> = sqlx::query_scalar(r#"EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) SELECT security_code, date, closing_price FROM estimate WHERE security_code = $1 AND ($2::date IS NULL OR (date <= $2 AND date >= $2 - 30)) ORDER BY date DESC LIMIT 1"#)
            .bind(symbol).bind(Option::<NaiveDate>::None).fetch_all(pool).await.expect("valuation EXPLAIN");
        let breadth: Vec<String> = sqlx::query_scalar(r#"EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) WITH endpoint AS (SELECT MAX(date) AS date FROM daily_stock_price_stats WHERE stock_exchange_market_id = $1 AND ($2::date IS NULL OR (date <= $2 AND date >= $2 - 30))) SELECT s.date FROM daily_stock_price_stats s CROSS JOIN endpoint e WHERE s.stock_exchange_market_id = $1 AND s.date <= e.date ORDER BY s.date DESC LIMIT $3"#)
            .bind(0_i32).bind(Option::<NaiveDate>::None).bind(20_i64).fetch_all(pool).await.expect("breadth EXPLAIN");
        let ranking: Vec<String> = sqlx::query_scalar(r#"EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) SELECT y.security_code FROM yield_rank y JOIN stocks s ON s.stock_symbol = y.security_code JOIN "DailyQuotes" q ON q."Serial" = y.daily_quotes_serial JOIN dividend d ON d.serial = y.dividend_serial WHERE y.date = (SELECT MAX(date) FROM yield_rank) AND s.stock_exchange_market_id IN (2,4) ORDER BY y.yield DESC, y.security_code ASC LIMIT 20"#)
            .fetch_all(pool).await.expect("ranking EXPLAIN");
        let screen_sql = format!(
            "EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) {SCREEN_STOCKS_SQL}\nORDER BY dividend_yield_percent DESC NULLS LAST, stock_symbol ASC\nLIMIT $9"
        );
        let screen: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(screen_sql.as_str()))
            .bind(0_i32)
            .bind(Option::<i32>::None)
            .bind(chrono::Local::now().date_naive())
            .bind(Option::<Decimal>::None)
            .bind(Option::<Decimal>::None)
            .bind(Option::<Decimal>::None)
            .bind(Option::<Decimal>::None)
            .bind(Option::<&str>::None)
            .bind(20_i64)
            .fetch_all(pool)
            .await
            .expect("screen EXPLAIN");

        for (name, plan) in [
            ("valuation", valuation),
            ("breadth", breadth),
            ("yield-ranking", ranking),
            ("screen", screen),
        ] {
            assert!(!plan.is_empty(), "{name} 應回傳 plan");
            println!("\n===== {name} =====\n{}", plan.join("\n"));
            if name == "screen" {
                assert!(
                    plan.join("\n")
                        .contains("yield_rank-security_code-date-desc-idx"),
                    "screen 每股最新殖利率必須走複合索引"
                );
            }
        }
    }
}
