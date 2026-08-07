use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::Row;

use crate::domain::performance::{DividendEvent, source::CagrSourceRepository};
use crate::infra::database;

/// `DailyQuotes."Date"` 的預設哨兵值下界。
///
/// 該欄位在部分資料列留有 `1970-01-01` 這類預設值（非真實交易日），
/// 若不排除，「最早報價日」與「涵蓋年數」都會被嚴重扭曲。
const SENTINEL_DATE_FLOOR: &str = "1990-01-01";

/// 判定疑似減資／分割的單日跳動門檻。
///
/// 台股漲跌幅限制為 ±10%，直覺上超過 11% 就該視為異常，但 2026-08-07 以近十年
/// 實際資料校準的結果顯示這個門檻完全不堪用 —— 會標記到全市場 45% 的股票，
/// 等於沒有標記。誤判來源主要有三類：興櫃與追蹤外國指數的 ETF／ETN 本就沒有
/// 漲跌幅限制、新上市初期不受限、以及**除息日未登錄**（`dividend` 表有 17,553
/// 筆除息日是 `'-'`，這些除權息造成的跳空無法被排除）。
///
/// 實測跳動幅度分布（十年、已排除可比對到的除權息日）：
///
/// | 幅度 | 檔數 |
/// |------|------|
/// | 11–20% | 1,113 |
/// | 20–30% | 412 |
/// | 30% 以上 | 553 |
///
/// 改採 30%：減資彌補虧損五成會讓參考價翻倍、減資三成約跳 43%，都遠在此門檻
/// 之上；而 11–20% 那一大段主要是上述誤判來源。
///
/// **已知限制**：小幅現金減資（例如減資一成，股價僅上調約 11%）不會被偵測到。
/// 這是刻意的取捨 —— 那種幅度對 CAGR 的扭曲本來就小，而為了抓它把門檻降下來，
/// 會讓旗標因為誤判太多而被使用者完全忽略。
const ANOMALY_JUMP_THRESHOLD: &str = "0.30";

/// CAGR 原始資料來源之 PostgreSQL 實作。
#[derive(Debug, Clone, Copy, Default)]
pub struct PgCagrSourceRepository;

impl PgCagrSourceRepository {
    /// 建立實例。
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl CagrSourceRepository for PgCagrSourceRepository {
    async fn fetch_trading_day_on_or_before(&self, date: NaiveDate) -> Result<Option<NaiveDate>> {
        let sql = r#"
            SELECT MAX("Date") AS trading_day
            FROM "DailyQuotes"
            WHERE "Date" <= $1 AND "Date" > $2::date
        "#;

        let row = sqlx::query(sql)
            .bind(date)
            .bind(SENTINEL_DATE_FLOOR)
            .fetch_one(database::get_connection())
            .await
            .context("Failed to fetch trading day on or before the given date")?;

        Ok(row.try_get::<Option<NaiveDate>, _>("trading_day")?)
    }

    async fn fetch_latest_trading_day(&self) -> Result<Option<NaiveDate>> {
        let sql = r#"
            SELECT MAX("Date") AS trading_day
            FROM "DailyQuotes"
            WHERE "Date" > $1::date
        "#;

        let row = sqlx::query(sql)
            .bind(SENTINEL_DATE_FLOOR)
            .fetch_one(database::get_connection())
            .await
            .context("Failed to fetch the latest trading day")?;

        Ok(row.try_get::<Option<NaiveDate>, _>("trading_day")?)
    }

    async fn fetch_active_symbols(&self) -> Result<Vec<String>> {
        let sql = r#"
            SELECT stock_symbol
            FROM stocks
            WHERE "SuspendListing" = false
            ORDER BY stock_symbol
        "#;

        let rows = sqlx::query(sql)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch active symbols")?;

        rows.into_iter()
            .map(|row| Ok(row.try_get::<String, _>("stock_symbol")?))
            .collect()
    }

    async fn fetch_first_quote_dates(&self) -> Result<Vec<(String, NaiveDate)>> {
        let sql = r#"
            SELECT stock_symbol, MIN("Date") AS first_date
            FROM "DailyQuotes"
            WHERE "Date" > $1::date AND "ClosingPrice" > 0
            GROUP BY stock_symbol
        "#;

        let rows = sqlx::query(sql)
            .bind(SENTINEL_DATE_FLOOR)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch first quote dates")?;

        rows.into_iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("stock_symbol")?,
                    row.try_get::<NaiveDate, _>("first_date")?,
                ))
            })
            .collect()
    }

    async fn fetch_closing_prices_on(&self, date: NaiveDate) -> Result<Vec<(String, Decimal)>> {
        let sql = r#"
            SELECT stock_symbol, "ClosingPrice"
            FROM "DailyQuotes"
            WHERE "Date" = $1 AND "ClosingPrice" > 0
        "#;

        let rows = sqlx::query(sql)
            .bind(date)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch closing prices on the given date")?;

        rows.into_iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("stock_symbol")?,
                    row.try_get::<Decimal, _>("ClosingPrice")?,
                ))
            })
            .collect()
    }

    async fn fetch_first_quote_within(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<(String, NaiveDate, Decimal)>> {
        // 區間刻意為左開右閉：`from`（統一對齊後的期初交易日）當天有報價的
        // 股票走一般路徑，這裡只處理當天沒有報價、需要套用寬限規則者。
        let sql = r#"
            SELECT DISTINCT ON (stock_symbol)
                   stock_symbol, "Date", "ClosingPrice"
            FROM "DailyQuotes"
            WHERE "Date" > $1 AND "Date" <= $2 AND "ClosingPrice" > 0
            ORDER BY stock_symbol, "Date"
        "#;

        let rows = sqlx::query(sql)
            .bind(from)
            .bind(to)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch first quote within the grace period")?;

        rows.into_iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("stock_symbol")?,
                    row.try_get::<NaiveDate, _>("Date")?,
                    row.try_get::<Decimal, _>("ClosingPrice")?,
                ))
            })
            .collect()
    }

    async fn fetch_dividend_events_since(&self, since: NaiveDate) -> Result<Vec<DividendEvent>> {
        // 兩個必要的防護，缺一結果就會系統性錯誤：
        //
        // 1. `has_detail` 去重：同一 (security_code, year) 可能同時存在
        //    `quarter = ''` 的年度彙總列與季度明細列（2026-08-07 實測 1,619
        //    組）。年度列是由明細聚合產生的，一起加總會讓季配息股算兩次。
        // 2. 除權息日欄位是 varchar 且含髒值（`'-'`、`'尚未公布'`，實測甚至
        //    有 `'1.39%'` 這種百分比字串）。這裡只用正則過濾掉明顯不合格式
        //    者以減少傳輸量，**絕不做 `::date` 轉型**——PostgreSQL 不保證
        //    WHERE 與 SELECT 的求值順序，轉型仍可能碰到髒值而整批失敗。
        //    真正的解析在 Rust 端進行，失敗者視為該日期不存在。
        let sql = r#"
            WITH has_detail AS (
                SELECT security_code, year
                FROM dividend
                WHERE quarter <> ''
                GROUP BY security_code, year
            )
            SELECT d.security_code,
                   d."ex-dividend_date1",
                   d."ex-dividend_date2",
                   d.cash_dividend,
                   d.stock_dividend
            FROM dividend d
            LEFT JOIN has_detail hd
                   ON hd.security_code = d.security_code AND hd.year = d.year
            WHERE (d.quarter <> '' OR hd.year IS NULL)
              AND (d.cash_dividend > 0 OR d.stock_dividend > 0)
              AND (
                    (d."ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$' AND d."ex-dividend_date1" >= $1)
                 OR (d."ex-dividend_date2" ~ '^\d{4}-\d{2}-\d{2}$' AND d."ex-dividend_date2" >= $1)
              )
        "#;

        let rows = sqlx::query(sql)
            .bind(since.format("%Y-%m-%d").to_string())
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch dividend events")?;

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let event = DividendEvent {
                stock_symbol: row.try_get::<String, _>("security_code")?,
                ex_dividend_date_cash: parse_ex_dividend_date(
                    &row.try_get::<String, _>("ex-dividend_date1")?,
                ),
                ex_dividend_date_stock: parse_ex_dividend_date(
                    &row.try_get::<String, _>("ex-dividend_date2")?,
                ),
                cash_dividend: row.try_get::<Decimal, _>("cash_dividend")?,
                stock_dividend: row.try_get::<Decimal, _>("stock_dividend")?,
            };

            // 兩個日期都解析失敗的事件無法定位於時間軸上，直接略過。
            if event.sort_key().is_some() {
                events.push(event);
            }
        }

        Ok(events)
    }

    async fn fetch_closing_prices_at(
        &self,
        pairs: &[(String, NaiveDate)],
    ) -> Result<Vec<(String, NaiveDate, Decimal)>> {
        if pairs.is_empty() {
            return Ok(Vec::new());
        }

        // 以 UNNEST 把數萬筆 (代號, 日期) 組合一次送進資料庫，
        // 比逐筆查詢少掉數萬次往返；JOIN 走 stock_symbol + Date 唯一索引。
        let symbols: Vec<String> = pairs.iter().map(|(symbol, _)| symbol.clone()).collect();
        let dates: Vec<NaiveDate> = pairs.iter().map(|(_, date)| *date).collect();

        let sql = r#"
            SELECT dq.stock_symbol, dq."Date", dq."ClosingPrice"
            FROM UNNEST($1::varchar[], $2::date[]) AS wanted(stock_symbol, quote_date)
            JOIN "DailyQuotes" dq
              ON dq.stock_symbol = wanted.stock_symbol
             AND dq."Date" = wanted.quote_date
            WHERE dq."ClosingPrice" > 0
        "#;

        let rows = sqlx::query(sql)
            .bind(&symbols)
            .bind(&dates)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch closing prices for the given symbol/date pairs")?;

        rows.into_iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("stock_symbol")?,
                    row.try_get::<NaiveDate, _>("Date")?,
                    row.try_get::<Decimal, _>("ClosingPrice")?,
                ))
            })
            .collect()
    }

    async fn fetch_anomaly_events(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<(String, NaiveDate)>> {
        // 本專案沒有記錄減資與股票分割，只能從價格序列反推：單日跳動超過
        // [`ANOMALY_JUMP_THRESHOLD`]、且當天沒有對應的除權息事件，即視為疑似異常。
        //
        // 回傳帶日期的事件（而非去重後的代號），呼叫端才能把單次查詢的結果依日期
        // 切分到八個期間各自判定。
        //
        // 比對除權息日時刻意用 `to_char(...)` 把日期轉成字串去比對 varchar 欄位，
        // 而不是把 varchar 轉成 date —— 後者會在髒值上整批失敗。
        let sql = r#"
            WITH px AS (
                SELECT stock_symbol,
                       "Date",
                       "ClosingPrice",
                       LAG("ClosingPrice") OVER (
                           PARTITION BY stock_symbol ORDER BY "Date"
                       ) AS prev_price
                FROM "DailyQuotes"
                WHERE "Date" BETWEEN $1 AND $2 AND "ClosingPrice" > 0
            ),
            jumps AS (
                SELECT stock_symbol, "Date"
                FROM px
                WHERE prev_price IS NOT NULL
                  AND prev_price > 0
                  AND ABS("ClosingPrice" / prev_price - 1) > $3::numeric
            )
            SELECT j.stock_symbol, j."Date"
            FROM jumps j
            WHERE NOT EXISTS (
                SELECT 1
                FROM dividend d
                WHERE d.security_code = j.stock_symbol
                  AND (
                        d."ex-dividend_date1" = to_char(j."Date", 'YYYY-MM-DD')
                     OR d."ex-dividend_date2" = to_char(j."Date", 'YYYY-MM-DD')
                  )
            )
        "#;

        let rows = sqlx::query(sql)
            .bind(from)
            .bind(to)
            .bind(ANOMALY_JUMP_THRESHOLD)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to detect anomaly events")?;

        rows.into_iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("stock_symbol")?,
                    row.try_get::<NaiveDate, _>("Date")?,
                ))
            })
            .collect()
    }
}

/// 解析除權息日字串。
///
/// 欄位型別為 `varchar(10)`，實務值除了 `YYYY-MM-DD` 之外還包含 `'-'`、
/// `'尚未公布'`、空字串，甚至 `'1.39%'` 這類明顯放錯欄位的百分比字串。
/// 一律以「解析失敗即視為沒有這個日期」處理，與 domain 層
/// `Dividend::is_eligible_for_date` 的既有語義一致。
fn parse_ex_dividend_date(raw: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 驗證髒值一律回傳 `None`，合法日期正常解析。
    #[test]
    fn test_parse_ex_dividend_date() {
        assert_eq!(
            parse_ex_dividend_date("2024-07-11"),
            NaiveDate::from_ymd_opt(2024, 7, 11)
        );
        assert_eq!(
            parse_ex_dividend_date(" 2024-07-11 "),
            NaiveDate::from_ymd_opt(2024, 7, 11)
        );

        // 以下皆為資料庫中實際出現過的髒值。
        assert_eq!(parse_ex_dividend_date("-"), None);
        assert_eq!(parse_ex_dividend_date("尚未公布"), None);
        assert_eq!(parse_ex_dividend_date(""), None);
        assert_eq!(parse_ex_dividend_date("1.39%"), None);
        assert_eq!(parse_ex_dividend_date("2024/07/11"), None);
    }
}
