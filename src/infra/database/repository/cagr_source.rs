use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::Row;

use crate::domain::performance::{CorporateAction, DividendEvent, source::CagrSourceRepository};
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

    async fn fetch_corporate_actions_since(
        &self,
        since: NaiveDate,
    ) -> Result<Vec<CorporateAction>> {
        // 比例非正數的列直接在 SQL 濾掉：那是登錄錯誤（0 會讓持股歸零、
        // 負數毫無意義），寧可當成「沒登錄」也不要算出錯誤的報酬率。
        let sql = r#"
            SELECT stock_symbol, effective_date, share_ratio, note
            FROM corporate_action
            WHERE effective_date > $1 AND share_ratio > 0
            ORDER BY stock_symbol, effective_date
        "#;

        let rows = sqlx::query(sql)
            .bind(since)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch corporate actions")?;

        rows.into_iter()
            .map(|row| {
                Ok(CorporateAction {
                    stock_symbol: row.try_get::<String, _>("stock_symbol")?,
                    effective_date: row.try_get::<NaiveDate, _>("effective_date")?,
                    share_ratio: row.try_get::<Decimal, _>("share_ratio")?,
                    note: row.try_get::<String, _>("note")?,
                })
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
            -- 已登錄的分割／減資是「已建模」的事件，模擬時會正確調整股數，
            -- 不該再標記為異常——否則旗標會一直亮著，使用者無從分辨哪些
            -- 才是真正未處理的問題。
            AND NOT EXISTS (
                SELECT 1
                FROM corporate_action ca
                WHERE ca.stock_symbol = j.stock_symbol
                  AND ca.effective_date = j."Date"
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
    use rust_decimal_macros::dec;

    /// 測試專用的假代號；真實市場不存在此代號。
    const FAKE_A: &str = "79979S1";
    /// 已下市的假代號，用於驗證母體排除規則。
    const FAKE_B: &str = "79979S2";

    /// 測試資料一律落在 1990 年初 —— 遠早於 `DailyQuotes` 實際涵蓋的 2012 年，
    /// 既不會與正式資料互相干擾，也仍在哨兵值 `1990-01-01` 之後。
    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(1990, 1, d).expect("測試日期應合法")
    }

    async fn cleanup() {
        let symbols = vec![FAKE_A.to_string(), FAKE_B.to_string()];
        let _ = sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#)
            .bind(&symbols)
            .execute(database::get_connection())
            .await;
        let _ = sqlx::query("DELETE FROM dividend WHERE security_code = ANY($1)")
            .bind(&symbols)
            .execute(database::get_connection())
            .await;
        let _ = sqlx::query("DELETE FROM stocks WHERE stock_symbol = ANY($1)")
            .bind(&symbols)
            .execute(database::get_connection())
            .await;
        let _ = sqlx::query("DELETE FROM corporate_action WHERE stock_symbol = ANY($1)")
            .bind(&symbols)
            .execute(database::get_connection())
            .await;
    }

    async fn insert_corporate_action(symbol: &str, date: NaiveDate, ratio: Decimal) {
        sqlx::query(
            r#"INSERT INTO corporate_action (stock_symbol, effective_date, action_type, share_ratio, note)
               VALUES ($1, $2, 'split', $3, '測試用')
               ON CONFLICT (stock_symbol, effective_date) DO UPDATE SET share_ratio = excluded.share_ratio"#,
        )
        .bind(symbol)
        .bind(date)
        .bind(ratio)
        .execute(database::get_connection())
        .await
        .expect("插入公司行動");
    }

    async fn insert_quote(symbol: &str, date: NaiveDate, close: Decimal) {
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice") VALUES ($1, $2, $3)"#,
        )
        .bind(date)
        .bind(symbol)
        .bind(close)
        .execute(database::get_connection())
        .await
        .expect("插入報價");
    }

    async fn insert_stock(symbol: &str, suspend: bool) {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, "SuspendListing")
               VALUES ($1, $1, $1, $2)
               ON CONFLICT (stock_symbol) DO UPDATE SET "SuspendListing" = excluded."SuspendListing""#,
        )
        .bind(symbol)
        .bind(suspend)
        .execute(database::get_connection())
        .await
        .expect("插入股票母檔");
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_dividend(
        symbol: &str,
        year: i32,
        quarter: &str,
        cash: Decimal,
        stock: Decimal,
        date1: &str,
        date2: &str,
    ) {
        sqlx::query(
            r#"INSERT INTO dividend (security_code, year, quarter, cash_dividend, stock_dividend,
                                     "ex-dividend_date1", "ex-dividend_date2")
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (security_code, year, quarter) DO UPDATE
                   SET cash_dividend = excluded.cash_dividend"#,
        )
        .bind(symbol)
        .bind(year)
        .bind(quarter)
        .bind(cash)
        .bind(stock)
        .bind(date1)
        .bind(date2)
        .execute(database::get_connection())
        .await
        .expect("插入股利");
    }

    /// 建立測試資料集。
    ///
    /// `79979S1` 的價格序列刻意安排兩次大跳動：01-03 那次有對應的除息日
    /// （不得標記為異常），01-05 那次沒有（必須標記）。
    async fn seed() {
        cleanup().await;

        insert_stock(FAKE_A, false).await;
        insert_stock(FAKE_B, true).await;

        // 1970-01-01 是資料庫中實際存在的預設哨兵值，必須被所有查詢排除。
        insert_quote(
            FAKE_A,
            NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            dec!(1),
        )
        .await;
        insert_quote(FAKE_A, day(2), dec!(100)).await;
        insert_quote(FAKE_A, day(3), dec!(160)).await;
        // 收盤價為零者視同無報價。
        insert_quote(FAKE_A, day(4), Decimal::ZERO).await;
        insert_quote(FAKE_A, day(5), dec!(300)).await;
        insert_quote(FAKE_A, day(8), dec!(310)).await;
        insert_quote(FAKE_B, day(3), dec!(50)).await;

        // 年度彙總列與同年季度明細列並存：只能採計明細，否則股利算兩次。
        insert_dividend(FAKE_A, 1990, "", dec!(5), Decimal::ZERO, "1990-01-03", "-").await;
        insert_dividend(
            FAKE_A,
            1990,
            "Q1",
            dec!(2),
            Decimal::ZERO,
            "1990-01-03",
            "-",
        )
        .await;
        // 除息日為髒值、除權日合法：仍應取得事件，但現金除息日為 None。
        insert_dividend(FAKE_A, 1991, "", dec!(3), dec!(1), "1.39%", "1991-03-01").await;
        // 兩個日期都是髒值 —— 無法定位於時間軸，必須整筆略過。
        insert_dividend(FAKE_B, 1990, "", dec!(1), Decimal::ZERO, "尚未公布", "-").await;
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_trading_day_and_symbol_queries() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_trading_day_and_symbol_queries：無資料庫連接");
            return;
        }
        seed().await;
        let repo = PgCagrSourceRepository::new();

        // 對齊到不晚於指定日的交易日；1990-01-04 收盤價為零但仍是交易日。
        assert_eq!(
            repo.fetch_trading_day_on_or_before(day(7))
                .await
                .expect("fetch_trading_day_on_or_before"),
            Some(day(5))
        );
        assert_eq!(
            repo.fetch_trading_day_on_or_before(day(3))
                .await
                .expect("fetch_trading_day_on_or_before"),
            Some(day(3))
        );
        // 哨兵值 1970-01-01 不得被當成交易日。
        assert_eq!(
            repo.fetch_trading_day_on_or_before(NaiveDate::from_ymd_opt(1989, 12, 31).unwrap())
                .await
                .expect("fetch_trading_day_on_or_before"),
            None
        );

        let latest = repo
            .fetch_latest_trading_day()
            .await
            .expect("fetch_latest_trading_day");
        assert!(latest.is_some_and(|d| d >= day(8)));

        let symbols = repo
            .fetch_active_symbols()
            .await
            .expect("fetch_active_symbols");
        assert!(symbols.iter().any(|s| s == FAKE_A));
        assert!(
            !symbols.iter().any(|s| s == FAKE_B),
            "已下市股票不得進入計算母體"
        );

        let first_quotes = repo
            .fetch_first_quote_dates()
            .await
            .expect("fetch_first_quote_dates");
        let first = first_quotes
            .iter()
            .find(|(symbol, _)| symbol == FAKE_A)
            .map(|(_, date)| *date);
        assert_eq!(first, Some(day(2)), "哨兵值 1970-01-01 必須被排除");

        cleanup().await;
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_price_queries_skip_zero_and_respect_ranges() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_price_queries_skip_zero_and_respect_ranges：無資料庫連接");
            return;
        }
        seed().await;
        let repo = PgCagrSourceRepository::new();

        let prices = repo
            .fetch_closing_prices_on(day(3))
            .await
            .expect("fetch_closing_prices_on");
        assert_eq!(
            prices
                .iter()
                .find(|(symbol, _)| symbol == FAKE_A)
                .map(|(_, price)| *price),
            Some(dec!(160))
        );

        // 收盤價為零者不得出現。
        let zero_day = repo
            .fetch_closing_prices_on(day(4))
            .await
            .expect("fetch_closing_prices_on");
        assert!(!zero_day.iter().any(|(symbol, _)| symbol == FAKE_A));

        // 寬限查詢的區間是左開右閉：期初日當天的報價不算，往後第一筆才算。
        let grace = repo
            .fetch_first_quote_within(day(2), day(6))
            .await
            .expect("fetch_first_quote_within");
        let hit = grace.iter().find(|(symbol, _, _)| symbol == FAKE_A);
        assert_eq!(
            hit.map(|(_, date, price)| (*date, *price)),
            Some((day(3), dec!(160)))
        );

        // 批次查價：命中者回傳，查無者靜默略過（不得整批失敗）。
        let pairs = vec![
            (FAKE_A.to_string(), day(5)),
            (FAKE_A.to_string(), day(6)),
            (FAKE_A.to_string(), day(4)),
        ];
        let at = repo
            .fetch_closing_prices_at(&pairs)
            .await
            .expect("fetch_closing_prices_at");
        assert_eq!(at.len(), 1);
        assert_eq!(at[0], (FAKE_A.to_string(), day(5), dec!(300)));

        // 空輸入不應送出查詢。
        assert!(
            repo.fetch_closing_prices_at(&[])
                .await
                .expect("fetch_closing_prices_at empty")
                .is_empty()
        );

        cleanup().await;
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_dividend_events_deduplicate_and_tolerate_dirty_dates() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!(
                "跳過 test_dividend_events_deduplicate_and_tolerate_dirty_dates：無資料庫連接"
            );
            return;
        }
        seed().await;
        let repo = PgCagrSourceRepository::new();

        let events = repo
            .fetch_dividend_events_since(day(1))
            .await
            .expect("fetch_dividend_events_since");
        let mine: Vec<&DividendEvent> = events
            .iter()
            .filter(|event| event.stock_symbol == FAKE_A || event.stock_symbol == FAKE_B)
            .collect();

        // 1990 年只能取到季度明細（現金 2），年度彙總列（現金 5）必須被去重掉，
        // 否則季配息股的股利會被計算兩次。
        let nineteen_ninety: Vec<&&DividendEvent> = mine
            .iter()
            .filter(|event| event.ex_dividend_date_cash == Some(day(3)))
            .collect();
        assert_eq!(nineteen_ninety.len(), 1, "年度彙總列與明細列不得同時採計");
        assert_eq!(nineteen_ninety[0].cash_dividend, dec!(2));

        // 除息日是 '1.39%' 這種髒值時只丟棄該日期，事件本身仍憑除權日成立。
        let dirty = mine
            .iter()
            .find(|event| event.ex_dividend_date_stock.is_some())
            .expect("應取得只有除權日的事件");
        assert_eq!(dirty.ex_dividend_date_cash, None);
        assert_eq!(
            dirty.ex_dividend_date_stock,
            NaiveDate::from_ymd_opt(1991, 3, 1)
        );

        // 兩個日期都是髒值的事件無法定位，整筆略過。
        assert!(
            !mine.iter().any(|event| event.stock_symbol == FAKE_B),
            "沒有任何可解析日期的事件不得回傳"
        );

        // since 之後才生效：1991 的事件仍在，1990 的已被濾掉。
        let later = repo
            .fetch_dividend_events_since(NaiveDate::from_ymd_opt(1991, 1, 1).unwrap())
            .await
            .expect("fetch_dividend_events_since later");
        let later_mine: Vec<&DividendEvent> = later
            .iter()
            .filter(|event| event.stock_symbol == FAKE_A)
            .collect();
        assert_eq!(later_mine.len(), 1);
        assert_eq!(later_mine[0].stock_dividend, dec!(1));

        cleanup().await;
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_anomaly_detection_ignores_jumps_explained_by_dividends() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!(
                "跳過 test_anomaly_detection_ignores_jumps_explained_by_dividends：無資料庫連接"
            );
            return;
        }
        seed().await;
        let repo = PgCagrSourceRepository::new();

        let events = repo
            .fetch_anomaly_events(day(2), day(8))
            .await
            .expect("fetch_anomaly_events");
        let mine: Vec<NaiveDate> = events
            .iter()
            .filter(|(symbol, _)| symbol == FAKE_A)
            .map(|(_, date)| *date)
            .collect();

        // 01-03 跳 60% 但當天有登錄除息日 → 可解釋，不標記；
        // 01-05 跳 87.5% 且無對應除權息 → 標記；
        // 01-08 僅跳 3.3%，未達 30% 門檻。
        assert_eq!(mine, vec![day(5)]);

        // 事件帶日期回傳，呼叫端才能依期間各自判定：縮小區間後該事件應消失。
        let narrowed = repo
            .fetch_anomaly_events(day(5), day(8))
            .await
            .expect("fetch_anomaly_events narrowed");
        assert!(
            !narrowed
                .iter()
                .any(|(symbol, date)| symbol == FAKE_A && *date == day(5)),
            "區間起點之後才計算跳動，起點當天不應再被標記"
        );

        cleanup().await;
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_corporate_actions_are_fetched_and_suppress_anomaly_flags() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!(
                "跳過 test_corporate_actions_are_fetched_and_suppress_anomaly_flags：無資料庫連接"
            );
            return;
        }
        seed().await;
        let repo = PgCagrSourceRepository::new();

        // 尚未登錄時，01-05 那次 87.5% 的跳動會被標記為異常。
        let before = repo
            .fetch_anomaly_events(day(2), day(8))
            .await
            .expect("fetch_anomaly_events");
        assert!(
            before
                .iter()
                .any(|(symbol, date)| symbol == FAKE_A && *date == day(5))
        );

        insert_corporate_action(FAKE_A, day(5), dec!(0.5)).await;

        let actions = repo
            .fetch_corporate_actions_since(day(1))
            .await
            .expect("fetch_corporate_actions_since");
        let mine: Vec<&CorporateAction> = actions
            .iter()
            .filter(|action| action.stock_symbol == FAKE_A)
            .collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].effective_date, day(5));
        assert_eq!(mine[0].share_ratio, dec!(0.5));

        // 登錄之後同一天不得再被視為異常——事件已建模，旗標應該熄滅。
        let after = repo
            .fetch_anomaly_events(day(2), day(8))
            .await
            .expect("fetch_anomaly_events again");
        assert!(
            !after
                .iter()
                .any(|(symbol, date)| symbol == FAKE_A && *date == day(5)),
            "已登錄的分割不該再被標記為疑似異常"
        );

        // since 之後才生效者才回傳。
        let later = repo
            .fetch_corporate_actions_since(day(5))
            .await
            .expect("fetch_corporate_actions_since later");
        assert!(!later.iter().any(|action| action.stock_symbol == FAKE_A));

        cleanup().await;
    }

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
