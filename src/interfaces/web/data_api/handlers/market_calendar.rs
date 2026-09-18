//! `/market/dividend-calendar` handler、其 UNION ALL 參數化 SQL、
//! 查詢區間解析與對應的資料庫列型別。

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;

use super::{
    analytical_decimal_to_f64, database_error, error_response, parse_optional_date, quarter_to_api,
};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{
    DividendCalendarEvent, DividendCalendarParams, DividendCalendarResponse, ErrorBody,
};

/// 查詢日期區間內的除權息與股利發放行事曆（§4.9）。
///
/// 資料來源是 `dividend` 表的四個**字串**日期欄位（除息日、除權日、現金
/// 發放日、股票發放日）；只有可解析為合法 `YYYY-MM-DD` 且落在查詢區間內
/// 的日期才輸出為事件，`-`、`尚未公布`、空字串等標記一律排除。同一筆股利
/// 公告若有多個日期落在區間內，輸出多筆事件（每筆一個 `event_type`）。
///
/// # Errors
///
/// 日期格式錯誤、`from` 晚於 `to`、區間超過 92 天、`event_type` 不在固定
/// enum 或 `limit` 超出 1–200 回 422；驗證失敗回 401；資料庫查詢失敗時
/// 記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/dividend-calendar", tag = "data-api", params(DividendCalendarParams), responses((status = 200, body = DividendCalendarResponse), (status = 401, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn dividend_calendar(Query(params): Query<DividendCalendarParams>) -> Response {
    let from = match parse_optional_date(params.from.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let to = match parse_optional_date(params.to.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    // 預設區間由 handler 明確帶入「台北本地今天」計算，與 screen_stocks
    // 相同；純函式 `resolve_calendar_range` 可用任意日期做 deterministic 測試。
    let (from, to) = match resolve_calendar_range(from, to, Local::now().date_naive()) {
        Ok(range) => range,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let event_type = params.event_type.as_deref().unwrap_or("all");
    if ![
        "ex_dividend",
        "ex_rights",
        "cash_payable",
        "stock_payable",
        "all",
    ]
    .contains(&event_type)
    {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "event_type 必須為 ex_dividend、ex_rights、cash_payable、stock_payable 或 all",
        );
    }
    let limit = params.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 200");
    }
    let rows: Result<Vec<CalendarEventRow>, _> = sqlx::query_as(DIVIDEND_CALENDAR_SQL)
        .bind(from)
        .bind(to)
        .bind(event_type)
        .bind(i64::from(limit))
        .fetch_all(database::get_connection())
        .await;
    match rows {
        Ok(rows) => Json(DividendCalendarResponse {
            // 混合事件沒有單一統計日期（§3.4），固定為 null；各事件日期
            // 放在每筆事件內。
            data_as_of: None,
            events: rows.into_iter().map(Into::into).collect(),
        })
        .into_response(),
        Err(error) => database_error(error),
    }
}

/// 股利行事曆的參數化 SQL：四段 UNION ALL，每段一種事件類型。
///
/// SQLx 0.9 要求字面 SQL（`&'static str`），因此四個日期欄位以 UNION ALL
/// 展開而非動態拼接；每段的 `event_type` 都是程式內字面值。日期欄位是
/// 字串且含 `-`、`尚未公布`、空字串、甚至 `1.39%` 等髒資料（P0-3 實測），
/// **不可直接 `col::date`**（單一髒值會讓整句報錯）——必須先以 regex 確認
/// 外觀合法再轉型；CASE 的惰性求值保證 regex 不符時不會執行轉型。
///
/// 綁定參數：
/// - `$1`：區間起日（含）；`BETWEEN` 同時排除 regex 不符產生的 NULL。
/// - `$2`：區間迄日（含）。
/// - `$3`：事件類型；`'all'` 時不過濾（哨兵展開與市場 id 的做法一致）。
/// - `$4`：回傳筆數上限。
///
/// 字串日期無法利用索引做範圍查詢；P0-4 EXPLAIN 實測四段平行 Seq Scan
/// （每段約 5 萬列）合計約 99ms，成本可接受，暫不建立 generated column。
const DIVIDEND_CALENDAR_SQL: &str = r#"
SELECT stock_symbol, name, event_type, event_date, year_of_dividend, quarter,
       cash_dividend, stock_dividend, total_dividend
FROM (
    SELECT d.security_code AS stock_symbol, s."Name" AS name,
           'ex_dividend' AS event_type,
           CASE WHEN d."ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$'
                THEN d."ex-dividend_date1"::date END AS event_date,
           d.year_of_dividend, d.quarter,
           d.cash_dividend, d.stock_dividend, d."sum" AS total_dividend
    FROM dividend d JOIN stocks s ON s.stock_symbol = d.security_code
    UNION ALL
    SELECT d.security_code, s."Name", 'ex_rights',
           CASE WHEN d."ex-dividend_date2" ~ '^\d{4}-\d{2}-\d{2}$'
                THEN d."ex-dividend_date2"::date END,
           d.year_of_dividend, d.quarter,
           d.cash_dividend, d.stock_dividend, d."sum"
    FROM dividend d JOIN stocks s ON s.stock_symbol = d.security_code
    UNION ALL
    SELECT d.security_code, s."Name", 'cash_payable',
           CASE WHEN d.payable_date1 ~ '^\d{4}-\d{2}-\d{2}$'
                THEN d.payable_date1::date END,
           d.year_of_dividend, d.quarter,
           d.cash_dividend, d.stock_dividend, d."sum"
    FROM dividend d JOIN stocks s ON s.stock_symbol = d.security_code
    UNION ALL
    SELECT d.security_code, s."Name", 'stock_payable',
           CASE WHEN d.payable_date2 ~ '^\d{4}-\d{2}-\d{2}$'
                THEN d.payable_date2::date END,
           d.year_of_dividend, d.quarter,
           d.cash_dividend, d.stock_dividend, d."sum"
    FROM dividend d JOIN stocks s ON s.stock_symbol = d.security_code
) events
WHERE event_date BETWEEN $1 AND $2
  AND ($3 = 'all' OR event_type = $3)
ORDER BY event_date ASC, stock_symbol ASC
LIMIT $4
"#;

/// 解析股利行事曆的實際查詢區間（§4.9）。
///
/// 規則（`today` 由 handler 傳入台北本地日，純函式便於 deterministic 測試）：
/// - `from` 未提供時預設查詢當日。
/// - `to` 未提供時預設 `from + 30` 天：除權息旺季集中在 6–9 月，一個月的
///   預設視窗足以回答「近期有哪些股票除息」而不會一次撈整季。
/// - `from` 不可晚於 `to`（§3.3）。
/// - `to - from` 上限 92 天（一季），避免 endpoint 被當成全表匯出介面。
///
/// # Errors
///
/// 區間顛倒、超過 92 天或日期加法溢位（極端如 `9999-12-31`）時回傳可直接
/// 作為 422 訊息的固定字串。
fn resolve_calendar_range(
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    today: NaiveDate,
) -> Result<(NaiveDate, NaiveDate), &'static str> {
    let from = from.unwrap_or(today);
    let to = match to {
        Some(value) => value,
        // `checked_add_days` 避免 `9999-12-31 + 30 天` 這類溢位 panic；
        // 溢位視為參數錯誤而非伺服器錯誤。
        None => from
            .checked_add_days(chrono::Days::new(30))
            .ok_or("日期超出可查詢範圍")?,
    };
    if from > to {
        return Err("from 不可晚於 to");
    }
    // `num_days` 以「兩端相減」計算：92 天上限指起迄差值，區間本身含首尾
    // 共 93 個日曆日（一季再加緩衝，涵蓋單一季度的所有除權息事件）。
    if (to - from).num_days() > 92 {
        return Err("查詢區間不可超過 92 天");
    }
    Ok((from, to))
}

/// 對應股利行事曆 UNION ALL 展開後的單一事件列（§4.9）。
#[derive(sqlx::FromRow)]
struct CalendarEventRow {
    /// 股票代號。
    stock_symbol: String,
    /// 股票名稱。
    name: String,
    /// 事件類型（SQL 內字面值之一）。
    event_type: String,
    /// 事件日期；`WHERE event_date BETWEEN` 已排除 NULL，可安全宣告非 Option。
    event_date: NaiveDate,
    /// 股利所屬年度。
    year_of_dividend: i32,
    /// 資料庫期間標記：空字串（年度）、`H1`／`H2` 或 `Q1`–`Q4`。
    quarter: String,
    /// 現金股利合計。
    cash_dividend: Option<Decimal>,
    /// 股票股利合計。
    stock_dividend: Option<Decimal>,
    /// 現金與股票股利總和（DB `"sum"`）。
    total_dividend: Option<Decimal>,
}

impl From<CalendarEventRow> for DividendCalendarEvent {
    /// 將事件列轉為 API DTO：期間標記依 §3.5 轉換（DB 空字串 → `A`），
    /// 金額欄位轉換失敗時帶股票代號記錄 log 並輸出 `null`。
    fn from(row: CalendarEventRow) -> Self {
        let symbol = row.stock_symbol.clone();
        let convert = |value, field| analytical_decimal_to_f64(value, &symbol, field);
        DividendCalendarEvent {
            event_date: row.event_date.to_string(),
            event_type: row.event_type,
            name: row.name,
            dividend_year: row.year_of_dividend,
            quarter: quarter_to_api(&row.quarter),
            cash_dividend: convert(row.cash_dividend, "cash_dividend"),
            stock_dividend: convert(row.stock_dividend, "stock_dividend"),
            total_dividend: convert(row.total_dividend, "total_dividend"),
            stock_symbol: row.stock_symbol,
        }
    }
}

#[cfg(test)]
mod tests {
    //! 行事曆查詢區間解析的 deterministic tests。

    use chrono::NaiveDate;

    use super::resolve_calendar_range;

    /// §4.9 行事曆區間解析：預設值、區間顛倒、92 天上限與溢位防護
    /// 都必須是 deterministic 純函式行為。
    #[test]
    fn calendar_range_resolution_follows_section_4_9() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 17).unwrap();
        // 兩者皆缺省：from = 當日、to = from + 30 天。
        assert_eq!(
            resolve_calendar_range(None, None, today),
            Ok((today, NaiveDate::from_ymd_opt(2026, 8, 16).unwrap()))
        );
        // 只給 from：to 以 from（而非當日）為基準加 30 天。
        let from = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        assert_eq!(
            resolve_calendar_range(Some(from), None, today),
            Ok((from, NaiveDate::from_ymd_opt(2026, 1, 31).unwrap()))
        );
        // 區間顛倒與超界：差 92 天是最後一個合法值，93 天必須被拒。
        let to_92 = NaiveDate::from_ymd_opt(2026, 4, 3).unwrap();
        let to_93 = NaiveDate::from_ymd_opt(2026, 4, 4).unwrap();
        assert_eq!(
            resolve_calendar_range(Some(from), Some(to_92), today)
                .unwrap()
                .1,
            to_92
        );
        assert_eq!(
            resolve_calendar_range(Some(from), Some(to_93), today),
            Err("查詢區間不可超過 92 天")
        );
        assert_eq!(
            resolve_calendar_range(Some(to_92), Some(from), today),
            Err("from 不可晚於 to")
        );
        // 極端日期的 to 預設值不可 panic，必須轉成參數錯誤。chrono 的
        // `NaiveDate` 值域超過西元 9999（`parse_optional_date` 的十字元
        // 格式實際到不了 `NaiveDate::MAX`），此斷言驗證 guard 讓函式對
        // 全值域皆為 total function，不留下 panic 路徑。
        assert_eq!(
            resolve_calendar_range(Some(NaiveDate::MAX), None, today),
            Err("日期超出可查詢範圍")
        );
    }
}
