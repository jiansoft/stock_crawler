//! CAGR 的疑似異常跳動掃描：單日跳動超過門檻、且前後兩筆報價之間沒有除權息或公司行動的事件，
//! 依代號雜湊分桶同時執行。

use anyhow::{Context, Result};
use chrono::NaiveDate;
use sqlx::Row;

use crate::infra::database;

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

/// 上市（櫃）後沒有漲跌幅限制的交易日數：前 5 個交易日。
const LISTING_FREE_DAYS: i64 = 5;

/// 疑似異常跳動掃描同時執行的桶數（各佔批次連線池一條連線）。
pub(super) const ANOMALY_BUCKETS: i32 = 4;

/// 掃描 `from`～`to` 的疑似異常跳動，只處理代號雜湊落在第 `bucket` 桶的股票；
/// `(1, 0)` 為不分桶。SQL 說明見 [`ANOMALY_EVENTS_SQL`]。
pub(super) async fn fetch_anomaly_bucket(
    from: NaiveDate,
    to: NaiveDate,
    (bucket_count, bucket): (i32, i32),
) -> Result<Vec<(String, NaiveDate)>> {
    let rows = sqlx::query(ANOMALY_EVENTS_SQL)
        .bind(from)
        .bind(to)
        .bind(ANOMALY_JUMP_THRESHOLD)
        .bind(LISTING_FREE_DAYS)
        .bind(bucket_count)
        .bind(bucket)
        // 每桶本來就要 2 秒上下，走批次連線池以免天天觸發 1 秒慢查詢警告。
        .fetch_all(database::get_batch_connection())
        .await
        .with_context(|| {
            format!("Failed to detect anomaly events in bucket {bucket}/{bucket_count}")
        })?;

    rows.into_iter()
        .map(|row| {
            Ok((
                row.try_get::<String, _>("stock_symbol")?,
                row.try_get::<NaiveDate, _>("Date")?,
            ))
        })
        .collect()
}

/// 掃描近十年全市場日K，靠 `DailyQuotes_symbol_Date_close_volume_idx`
/// （含收盤價與成交量）走 index-only scan，見 etc/sql/daily_quote.sql。
///
/// 本專案沒有記錄減資與股票分割，只能從價格序列反推：單日跳動超過
/// [`ANOMALY_JUMP_THRESHOLD`]、且兩筆報價之間沒有對應的除權息或公司行動，即視為疑似異常。
///
/// 「之間」是指前一筆報價之後、這一筆（含）之前：事件生效日不一定剛好有報價
/// （1529 2018-12-22 補班日缺報價、2314 2016-09-28 颱風休市），只比對同一天會誤判。
/// 上市（櫃）後前 [`LISTING_FREE_DAYS`] 筆報價沒有漲跌幅限制，期間的大幅跳動是真實價格，
/// 也不算異常（6637 2024-06-19、7810 2025-12-24）。
///
/// 只看有成交的報價：「缺漏補齊」在停牌期間寫入沿用前收的零量列，那不是成交價。
/// 1538 2024-12-30 減資恢復買賣當天沒有成交、補值列還是停牌前價格，
/// 若把它當成前一筆，跳動會落在隔天而對不上生效日。
///
/// 回傳帶日期的事件（而非去重後的代號），呼叫端才能把單次查詢的結果依日期
/// 切分到八個期間各自判定。
///
/// 比對除權息日時刻意用 `to_char(...)` 把日期轉成字串去比對 varchar 欄位，
/// 而不是把 varchar 轉成 date —— 後者會在髒值上整批失敗。
///
/// 參數：`$1`～`$2` 掃描期間、`$3` 跳動門檻、`$4` 上市初期豁免筆數、
/// `$5` 桶數、`$6` 桶號（`$5 = 1` 為不分桶）。
const ANOMALY_EVENTS_SQL: &str = r#"
    WITH px AS (
        SELECT stock_symbol,
               "Date",
               "ClosingPrice",
               -- 依索引的 (stock_symbol, "Date" DESC) 順序開窗，LEAD 即前一個交易日，
               -- 省掉每檔重新排序。
               LEAD("ClosingPrice") OVER w AS prev_price,
               LEAD("Date") OVER w AS prev_date
        FROM "DailyQuotes"
        WHERE "Date" BETWEEN $1 AND $2 AND "ClosingPrice" > 0 AND "TradingVolume" > 0
          -- 分桶：轉 bigint 再取絕對值，避免 hashtext 回傳 int 最小值時 abs() 溢位。
          AND ($5::int = 1 OR abs(hashtext(stock_symbol)::bigint) % $5::int = $6::int)
        WINDOW w AS (PARTITION BY stock_symbol ORDER BY "Date" DESC)
    ),
    jumps AS (
        SELECT stock_symbol, "Date", prev_date
        FROM px
        WHERE prev_price IS NOT NULL
          AND prev_price > 0
          -- 以乘法比較，避免對每一筆日K做 numeric 除法。
          AND ABS("ClosingPrice" - prev_price) > $3::numeric * prev_price
    )
    SELECT j.stock_symbol, j."Date"
    FROM jumps j
    WHERE NOT EXISTS (
        SELECT 1
        FROM dividend d
        WHERE d.security_code = j.stock_symbol
          AND (
                (d."ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$'
                 AND d."ex-dividend_date1" > to_char(j.prev_date, 'YYYY-MM-DD')
                 AND d."ex-dividend_date1" <= to_char(j."Date", 'YYYY-MM-DD'))
             OR (d."ex-dividend_date2" ~ '^\d{4}-\d{2}-\d{2}$'
                 AND d."ex-dividend_date2" > to_char(j.prev_date, 'YYYY-MM-DD')
                 AND d."ex-dividend_date2" <= to_char(j."Date", 'YYYY-MM-DD'))
          )
    )
    -- 已登錄的分割／減資是「已建模」的事件，模擬時會正確調整股數，
    -- 不該再標記為異常——否則旗標會一直亮著，使用者無從分辨哪些
    -- 才是真正未處理的問題。
    AND NOT EXISTS (
        SELECT 1
        FROM corporate_action ca
        WHERE ca.stock_symbol = j.stock_symbol
          AND ca.effective_date > j.prev_date
          AND ca.effective_date <= j."Date"
    )
    -- 上市（櫃）初期：這一筆之前的有效報價不足 LISTING_FREE_DAYS 筆。
    AND (
        SELECT count(*)
        FROM (
            SELECT 1
            FROM "DailyQuotes" e
            WHERE e.stock_symbol = j.stock_symbol
              AND e."Date" > '1970-01-01'
              AND e."Date" < j."Date"
              AND e."ClosingPrice" > 0
              AND e."TradingVolume" > 0
            LIMIT $4
        ) earlier
    ) >= $4
"#;
