use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::Row;

use self::anomaly::{ANOMALY_BUCKETS, fetch_anomaly_bucket};
use crate::domain::performance::{
    CorporateAction, CorporateActionType, DividendEvent, source::CagrSourceRepository,
};
use crate::infra::database;

mod anomaly;

/// `DailyQuotes."Date"` 的預設哨兵值下界。
///
/// 該欄位在部分資料列留有 `1970-01-01` 這類預設值（非真實交易日），
/// 若不排除，「最早報價日」與「涵蓋年數」都會被嚴重扭曲。
const SENTINEL_DATE_FLOOR: &str = "1990-01-01";

/// CAGR 原始資料來源之 PostgreSQL 實作。
#[derive(Debug, Clone, Copy, Default)]
pub struct PgCagrSourceRepository;

impl PgCagrSourceRepository {
    /// 建立實例。
    pub fn new() -> Self {
        Self
    }

    /// 取得指定股票在指定日期的單日收盤價變動率（%）。
    ///
    /// 供減資待辦清單標示跳動方向與幅度：正值代表價格上跳（減資的典型特徵），
    /// 負值代表下跳（較可能是分割）。
    ///
    /// 刻意不放進 [`CagrSourceRepository`] trait：這是報表輔助資訊，
    /// CAGR 的計算流程並不需要它，放進 trait 會讓所有測試替身被迫實作。
    ///
    /// 查無前一交易日報價（例如該日是該股第一天有資料）時回傳 `None`。
    pub async fn fetch_single_day_change_percent(
        &self,
        stock_symbol: &str,
        date: NaiveDate,
    ) -> Result<Option<Decimal>> {
        let sql = r#"
            WITH px AS (
                SELECT "Date",
                       "ClosingPrice",
                       LAG("ClosingPrice") OVER (ORDER BY "Date") AS prev_price
                FROM "DailyQuotes"
                WHERE stock_symbol = $1 AND "Date" <= $2 AND "ClosingPrice" > 0
                ORDER BY "Date" DESC
                LIMIT 2
            )
            SELECT ("ClosingPrice" / prev_price - 1) * 100 AS change_percent
            FROM px
            WHERE "Date" = $2 AND prev_price IS NOT NULL AND prev_price > 0
        "#;

        let value: Option<Decimal> = sqlx::query_scalar(sql)
            .bind(stock_symbol)
            .bind(date)
            .fetch_optional(database::get_connection())
            .await
            .context("Failed to fetch single day change percent")?
            .flatten();

        Ok(value)
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
        // 對整張 DailyQuotes GROUP BY 要掃完全部報價（pi 上 1.5～2.8 秒）；改成逐檔沿
        // (stock_symbol, "Date") 索引找第一筆，只讀約 2 千次索引，正式庫實測 975ms → 35ms。
        // 只查計算母體，已下市股票本來就用不到。
        let sql = r#"
            SELECT s.stock_symbol, f.first_date
            FROM stocks s
            CROSS JOIN LATERAL (
                SELECT q."Date" AS first_date
                FROM "DailyQuotes" q
                WHERE q.stock_symbol = s.stock_symbol
                  AND q."Date" > $1::date
                  AND q."ClosingPrice" > 0
                ORDER BY q."Date"
                LIMIT 1
            ) f
            WHERE s."SuspendListing" = false
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
            // 全市場近萬組除息日查價，計畫是 nested loop + index-only scan 已無可改；
            // 05:40 冷資料要約 1.8 秒，走批次連線池以免每次觸發 1 秒慢查詢警告。
            .fetch_all(database::get_batch_connection())
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
            SELECT stock_symbol, effective_date, action_type, share_ratio, note
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
                let share_ratio = row.try_get::<Decimal, _>("share_ratio")?;
                let action_type =
                    CorporateActionType::from_db_value(row.try_get::<&str, _>("action_type")?)
                        .unwrap_or_else(|| CorporateActionType::infer_from_ratio(share_ratio));

                Ok(CorporateAction {
                    stock_symbol: row.try_get::<String, _>("stock_symbol")?,
                    effective_date: row.try_get::<NaiveDate, _>("effective_date")?,
                    action_type,
                    share_ratio,
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
        // 依股票代號雜湊分成 ANOMALY_BUCKETS 桶同時跑：瓶頸是近十年約 478 萬筆日K 的
        // 開窗計算，單一語句只用得到一顆 CPU（2026-10-09 正式庫實測 4.8～5.0 秒 →
        // 4 桶 2.1 秒，結果逐筆相同；8 桶不再變快，每桶仍要掃過整個索引）。
        let buckets = (0..ANOMALY_BUCKETS)
            .map(|bucket| fetch_anomaly_bucket(from, to, (ANOMALY_BUCKETS, bucket)));
        let results = futures::future::try_join_all(buckets)
            .await
            .context("Failed to detect anomaly events")?;

        Ok(results.into_iter().flatten().collect())
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
mod tests;
