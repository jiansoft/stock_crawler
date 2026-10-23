//! 資料健康檢查的 PostgreSQL 量測實作。

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::{
    domain::{
        health::{
            DataHealthRepository, DataHealthSnapshot, DerivedTableLatest, EtfNavDayCount,
            ForwardReturnGap, ValuationRatioZeroCount,
        },
        quote::entity::MarketTradedCount,
    },
    infra::database::{self, table::daily_quote},
};

/// 每日收盤後會更新的衍生資料表，與各自的日期欄位。
///
/// 表名與欄位名無法用參數綁定，只能寫死在這裡；新增衍生表時同步加入。
const DERIVED_TABLES: [(&str, &str); 14] = [
    ("estimate", "SELECT max(date) FROM estimate"),
    ("yield_rank", "SELECT max(date) FROM yield_rank"),
    (
        "daily_stock_price_stats",
        "SELECT max(date) FROM daily_stock_price_stats",
    ),
    (
        "last_daily_quotes",
        "SELECT max(date) FROM last_daily_quotes",
    ),
    ("stock_cagr", "SELECT max(date) FROM stock_cagr"),
    ("qfii_history", "SELECT max(date) FROM qfii_history"),
    (
        "daily_technical_signal",
        "SELECT max(date) FROM daily_technical_signal",
    ),
    (
        "daily_relative_strength",
        "SELECT max(date) FROM daily_relative_strength",
    ),
    ("chip_daily", "SELECT max(date) FROM chip_daily"),
    // 只抓持股、來源只有當天，停更就補不回來，要盡早發現。
    ("broker_flow", "SELECT max(date) FROM broker_flow"),
    (
        "daily_market_valuation",
        "SELECT max(date) FROM daily_market_valuation",
    ),
    // 17:30 ETF 專區每日更新：含息績效與清單頁快照（計算基準日都是最後一個有成交的交易日）。
    (
        "etf_performance",
        "SELECT max(calc_date) FROM etf_performance",
    ),
    ("etf_snapshot", "SELECT max(quote_date) FROM etf_snapshot"),
    // 基準日要再過 5 個交易日才有 5 日報酬，看「有 5 日報酬的最新基準日」。
    (
        "daily_forward_return",
        "SELECT max(date) FROM daily_forward_return WHERE return_5d IS NOT NULL",
    ),
];

/// 短天期缺漏的列數占該基準日全部列數至少這個百分比才列出。
///
/// 回補範圍沒涵蓋到時整天都缺（接近 100%）；個股停牌、債券 ETF 不是每天成交，之後第 N 個
/// 交易日沒有報價，會造成零星缺值（2021-12-30 有 154 檔上櫃債券 ETF，約 8%），不算異常。
const FORWARD_RETURN_GAP_MIN_PERCENT: i64 = 20;

/// 從最早有 5 日報酬的基準日起，「較短天期為空、較長天期有值」的列數占比達門檻的基準日。
///
/// 更早的基準日本來就只回補了長天期（例如只為了相對強度補 240 日報酬），不列入。
const FORWARD_RETURN_GAPS_SQL: &str = r#"
WITH start AS (
    SELECT min(date) AS first_date FROM daily_forward_return WHERE return_5d IS NOT NULL
),
flagged AS (
    SELECT r.date,
           (r.return_5d IS NULL
            AND coalesce(r.return_20d, r.return_60d, r.return_120d, r.return_240d) IS NOT NULL)
           OR (r.return_20d IS NULL
               AND coalesce(r.return_60d, r.return_120d, r.return_240d) IS NOT NULL)
           OR (r.return_60d IS NULL AND coalesce(r.return_120d, r.return_240d) IS NOT NULL)
           OR (r.return_120d IS NULL AND r.return_240d IS NOT NULL) AS gap
    FROM daily_forward_return AS r, start
    WHERE r.date >= start.first_date
)
SELECT date, count(*) FILTER (WHERE gap)
FROM flagged
GROUP BY date
HAVING count(*) FILTER (WHERE gap) * 100 >= count(*) * $1
ORDER BY date
"#;

/// 檢查期間內有收盤價、前 400 天內已有至少 5 筆報價，5 日均線卻仍是 0 的列數。
const MISSING_MOVING_AVERAGES_SQL: &str = r#"
SELECT count(*)
FROM "DailyQuotes" AS d
WHERE d."Date" BETWEEN $1 AND $2
  AND d."ClosingPrice" > 0
  AND d."MovingAverage5" = 0
  AND (
      SELECT count(*)
      FROM (
          SELECT 1
          FROM "DailyQuotes" AS p
          WHERE p.stock_symbol = d.stock_symbol
            AND p."Date" <= d."Date"
            AND p."Date" >= d."Date" - 400
          LIMIT 5
      ) AS recent
  ) = 5
"#;

/// 漲跌幅與「漲跌 ÷ 參考價（收盤 − 漲跌）」不符的列數；漲跌為 0 時漲跌幅也必須是 0。
const CHANGE_RANGE_MISMATCHES_SQL: &str = r#"
SELECT count(*)
FROM "DailyQuotes"
WHERE "Date" BETWEEN $1 AND $2
  AND CASE
          WHEN "Change" = 0 OR "ClosingPrice" - "Change" <= 0 THEN "ChangeRange" <> 0
          ELSE abs("ChangeRange" - "Change" / ("ClosingPrice" - "Change") * 100) > 0.01
      END
"#;

/// `year`／`month`／`day` 與日期不符的列數（補值列曾漏填成 0）。
const MISDATED_ROWS_SQL: &str = r#"
SELECT count(*)
FROM "DailyQuotes"
WHERE "Date" BETWEEN $1 AND $2
  AND (year, month, day) IS DISTINCT FROM (
      date_part('year', "Date")::int,
      date_part('month', "Date")::int,
      date_part('day', "Date")::int
  )
"#;

/// 連續買賣超天數還沒計算的籌碼列數：每日入庫後的重算失敗時，排行、選股與回測會把它們當成 0。
const CHIP_STREAK_MISSING_SQL: &str = r#"
SELECT count(*)
FROM chip_daily
WHERE "date" BETWEEN $1 AND $2
  AND (foreign_streak IS NULL OR trust_streak IS NULL OR dealer_streak IS NULL OR total_streak IS NULL)
"#;

/// 近五年（不含今年）發放、除權息日仍是「尚未公布」的股利列數。
///
/// 更早的佔位列（1990 年代的配股）交易所已查不到實際日期，列出來也無從處理。
const STALE_DIVIDEND_PLACEHOLDERS_SQL: &str = r#"
SELECT count(*)
FROM dividend
WHERE year < $1 AND year >= $1 - 5
  AND ("ex-dividend_date1" = '尚未公布' OR "ex-dividend_date2" = '尚未公布')
"#;

/// 年度合計列與各期明細加總不符的組數（只比對有明細的年度）。
const DIVIDEND_TOTAL_MISMATCHES_SQL: &str = r#"
WITH details AS (
    SELECT security_code, year,
           sum(cash_dividend) AS cash_dividend,
           sum(stock_dividend) AS stock_dividend
    FROM dividend
    WHERE quarter <> ''
    GROUP BY security_code, year
)
SELECT count(*)
FROM dividend AS total
JOIN details USING (security_code, year)
WHERE total.quarter = ''
  AND (abs(total.cash_dividend - details.cash_dividend) > 0.0001
       OR abs(total.stock_dividend - details.stock_dividend) > 0.0001)
"#;

/// 每個交易日、每個市場有成交的個股（不含 ETF）中，本益比與股價淨值比為 0 的列數。
const VALUATION_RATIO_ZEROS_SQL: &str = r#"
SELECT q."Date", s.stock_exchange_market_id,
       count(*),
       count(*) FILTER (WHERE q."PriceEarningRatio" = 0),
       count(*) FILTER (WHERE q."price-to-book_ratio" = 0)
FROM "DailyQuotes" q
JOIN stocks s ON s.stock_symbol = q.stock_symbol
WHERE q."Date" BETWEEN $1 AND $2
  AND s.stock_exchange_market_id IN (2, 4)
  AND q."ClosingPrice" > 0
  AND q.stock_symbol !~ '^00'
GROUP BY 1, 2
ORDER BY 1, 2
"#;

/// 每天的 ETF 淨值列數與正式淨值還沒回填的列數。
const ETF_NAV_COUNTS_SQL: &str = r#"
SELECT date, count(*), count(*) FILTER (WHERE nav IS NULL)
FROM etf_daily_nav
WHERE date BETWEEN $1 AND $2
GROUP BY date
ORDER BY date
"#;

/// 清單頁上的 ETF 中，基本資料在 `$1` 之前就沒再更新的代號；還沒有基本資料的，
/// 只列出淨值在 `$1` 之前就出現（上市已超過一次週更）的，剛上市的等下一次週六更新。
const STALE_ETF_PROFILES_SQL: &str = r#"
SELECT s.stock_symbol
FROM etf_snapshot AS s
LEFT JOIN etf_profile AS p ON p.stock_symbol = s.stock_symbol
WHERE CASE
          WHEN p.stock_symbol IS NULL THEN
              (SELECT min(n.date) FROM etf_daily_nav AS n WHERE n.stock_symbol = s.stock_symbol) < $1
          ELSE p.updated_at < $1::date
      END
ORDER BY s.stock_symbol
"#;

/// 基於 PostgreSQL 的資料健康檢查量測。
#[derive(Debug, Default, Clone, Copy)]
pub struct PgDataHealthRepository;

impl PgDataHealthRepository {
    /// 建立新的量測實例。
    pub fn new() -> Self {
        Self
    }
}

/// 執行只回傳一個 `count(*)` 的查詢。
async fn count_between(
    sql: &'static str,
    from: NaiveDate,
    to: NaiveDate,
    label: &str,
) -> Result<i64> {
    sqlx::query_scalar(sql)
        .bind(from)
        .bind(to)
        .fetch_one(database::get_connection())
        .await
        .with_context(|| format!("Failed to count {label} {from}~{to}"))
}

#[async_trait]
impl DataHealthRepository for PgDataHealthRepository {
    async fn fetch_snapshot(
        &self,
        baseline_from: NaiveDate,
        from: NaiveDate,
        to: NaiveDate,
        current_year: i32,
        profile_due: NaiveDate,
    ) -> Result<DataHealthSnapshot> {
        let pool = database::get_connection();

        let market_counts = daily_quote::fetch_market_traded_counts(baseline_from, to)
            .await?
            .into_iter()
            .map(|(date, market_id, traded)| MarketTradedCount {
                date,
                market_id,
                traded,
            })
            .collect();

        let quoted_dates = sqlx::query_scalar(
            r#"SELECT DISTINCT "Date" FROM "DailyQuotes" WHERE "Date" BETWEEN $1 AND $2 ORDER BY 1"#,
        )
        .bind(from)
        .bind(to)
        .fetch_all(pool)
        .await
        .with_context(|| format!("Failed to fetch quoted dates {from}~{to}"))?;

        let mut derived_tables = Vec::with_capacity(DERIVED_TABLES.len());
        for (table, sql) in DERIVED_TABLES {
            let latest: Option<NaiveDate> = sqlx::query_scalar(sql)
                .fetch_one(pool)
                .await
                .with_context(|| format!("Failed to fetch latest date of {table}"))?;
            derived_tables.push(DerivedTableLatest { table, latest });
        }

        let stale_dividend_placeholders = sqlx::query_scalar(STALE_DIVIDEND_PLACEHOLDERS_SQL)
            .bind(current_year)
            .fetch_one(pool)
            .await
            .context("Failed to count stale dividend placeholders")?;

        let dividend_total_mismatches = sqlx::query_scalar(DIVIDEND_TOTAL_MISMATCHES_SQL)
            .fetch_one(pool)
            .await
            .context("Failed to count dividend total mismatches")?;

        let valuation_ratio_zeros =
            sqlx::query_as::<_, (NaiveDate, i32, i64, i64, i64)>(VALUATION_RATIO_ZEROS_SQL)
                .bind(from)
                .bind(to)
                .fetch_all(pool)
                .await
                .with_context(|| format!("Failed to count zero valuation ratios {from}~{to}"))?
                .into_iter()
                .map(
                    |(date, market_id, rows, pe_zero, pb_zero)| ValuationRatioZeroCount {
                        date,
                        market_id,
                        rows,
                        pe_zero,
                        pb_zero,
                    },
                )
                .collect();

        let forward_return_gaps = sqlx::query_as::<_, (NaiveDate, i64)>(FORWARD_RETURN_GAPS_SQL)
            .bind(FORWARD_RETURN_GAP_MIN_PERCENT)
            .fetch_all(pool)
            .await
            .context("Failed to find forward return gaps")?
            .into_iter()
            .map(|(date, rows)| ForwardReturnGap { date, rows })
            .collect();

        let etf_nav_counts = sqlx::query_as::<_, (NaiveDate, i64, i64)>(ETF_NAV_COUNTS_SQL)
            .bind(baseline_from)
            .bind(to)
            .fetch_all(pool)
            .await
            .with_context(|| format!("Failed to count ETF NAV rows {baseline_from}~{to}"))?
            .into_iter()
            .map(|(date, rows, missing_nav)| EtfNavDayCount {
                date,
                rows,
                missing_nav,
            })
            .collect();
        let stale_etf_profiles = sqlx::query_scalar(STALE_ETF_PROFILES_SQL)
            .bind(profile_due)
            .fetch_all(pool)
            .await
            .context("Failed to find stale ETF profiles")?;

        let latest_revenue_month = sqlx::query_scalar(r#"SELECT max("Date") FROM "Revenue""#)
            .fetch_one(pool)
            .await
            .context("Failed to fetch latest revenue month")?;
        let latest_holder_week =
            sqlx::query_scalar(r#"SELECT max("date") FROM holder_distribution"#)
                .fetch_one(pool)
                .await
                .context("Failed to fetch latest holder distribution week")?;
        let latest_insider_month =
            sqlx::query_scalar(r#"SELECT max("month") FROM insider_holding"#)
                .fetch_one(pool)
                .await
                .context("Failed to fetch latest insider holding month")?;

        Ok(DataHealthSnapshot {
            market_counts,
            quoted_dates,
            missing_moving_averages: count_between(
                MISSING_MOVING_AVERAGES_SQL,
                from,
                to,
                "missing moving averages",
            )
            .await?,
            change_range_mismatches: count_between(
                CHANGE_RANGE_MISMATCHES_SQL,
                from,
                to,
                "change range mismatches",
            )
            .await?,
            misdated_rows: count_between(MISDATED_ROWS_SQL, from, to, "misdated rows").await?,
            chip_streak_missing: count_between(
                CHIP_STREAK_MISSING_SQL,
                from,
                to,
                "chip streak missing",
            )
            .await?,
            derived_tables,
            stale_dividend_placeholders,
            dividend_total_mismatches,
            latest_revenue_month,
            latest_holder_week,
            latest_insider_month,
            valuation_ratio_zeros,
            forward_return_gaps,
            etf_nav_counts,
            stale_etf_profiles,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每一條量測 SQL 都要能在測試庫（由 `etc/sql` 建立）上執行。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn fetch_snapshot_runs_every_query() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 fetch_snapshot_runs_every_query：無資料庫連接");
            return;
        }

        let day = |d| NaiveDate::from_ymd_opt(2026, 4, d).expect("日期應合法");
        let snapshot = PgDataHealthRepository::new()
            .fetch_snapshot(day(1), day(24), day(30), 2026, day(21))
            .await
            .expect("量測應成功");

        assert_eq!(snapshot.derived_tables.len(), DERIVED_TABLES.len());
        assert!(snapshot.missing_moving_averages >= 0);
        assert!(snapshot.chip_streak_missing >= 0);
    }

    /// ETF 淨值依日期計數；基本資料停更或上市已久仍沒有基本資料的才列出。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn etf_queries_count_nav_rows_and_find_stale_profiles() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 etf_queries_count_nav_rows_and_find_stale_profiles：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        let mut tx = pool.begin().await.expect("開交易應成功");
        // 交易內清空，測完 rollback，不影響其他測試的資料。
        for sql in [
            "DELETE FROM etf_snapshot",
            "DELETE FROM etf_profile",
            "DELETE FROM etf_daily_nav",
        ] {
            sqlx::query(sql)
                .execute(&mut *tx)
                .await
                .expect("清空應成功");
        }
        let day = |d| NaiveDate::from_ymd_opt(2021, 3, d).expect("日期應合法");
        for (symbol, date, nav) in [
            ("79971", day(1), Some(10.0)),
            ("79972", day(1), None),
            ("79971", day(2), Some(10.1)),
            ("79973", day(9), Some(9.9)),
        ] {
            sqlx::query(
                "INSERT INTO etf_daily_nav (date, stock_symbol, estimated_nav, nav) VALUES ($1, $2, $3, $3)",
            )
            .bind(date)
            .bind(symbol)
            .bind(nav)
            .execute(&mut *tx)
            .await
            .expect("寫入淨值應成功");
        }
        // 79971 剛更新、79972 停更、79973 剛上市還沒有基本資料（不列）、79974 上市已久沒有基本資料。
        for (symbol, updated) in [("79971", "2021-03-09"), ("79972", "2021-03-01")] {
            sqlx::query("INSERT INTO etf_profile (stock_symbol, updated_at) VALUES ($1, $2::date)")
                .bind(symbol)
                .bind(updated)
                .execute(&mut *tx)
                .await
                .expect("寫入基本資料應成功");
        }
        sqlx::query(
            "INSERT INTO etf_daily_nav (date, stock_symbol, nav) VALUES ('2021-02-01', '79974', 10)",
        )
        .execute(&mut *tx)
        .await
        .expect("寫入淨值應成功");
        for symbol in ["79971", "79972", "79973", "79974"] {
            sqlx::query(
                "INSERT INTO etf_snapshot (stock_symbol, name, market_id) VALUES ($1, $1, 2)",
            )
            .bind(symbol)
            .execute(&mut *tx)
            .await
            .expect("寫入快照應成功");
        }

        let counts: Vec<(NaiveDate, i64, i64)> = sqlx::query_as(ETF_NAV_COUNTS_SQL)
            .bind(day(1))
            .bind(day(9))
            .fetch_all(&mut *tx)
            .await
            .expect("計數應成功");
        let stale: Vec<String> = sqlx::query_scalar(STALE_ETF_PROFILES_SQL)
            .bind(day(5))
            .fetch_all(&mut *tx)
            .await
            .expect("查詢應成功");
        tx.rollback().await.expect("rollback 應成功");

        assert_eq!(counts, [(day(1), 2, 1), (day(2), 1, 0), (day(9), 1, 0)]);
        assert_eq!(stale, ["79972", "79974"]);
    }

    /// 短天期缺漏占比達門檻的基準日要列出；零星缺值與最早有 5 日報酬之前的基準日不列入。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn forward_return_gaps_flag_whole_day_holes_only() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 forward_return_gaps_flag_whole_day_holes_only：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        let mut tx = pool.begin().await.expect("開交易應成功");
        // 交易內清空，測完 rollback，不影響其他測試的資料。
        sqlx::query("DELETE FROM daily_forward_return")
            .execute(&mut *tx)
            .await
            .expect("清空應成功");
        let insert = r#"
INSERT INTO daily_forward_return (date, stock_symbol, return_5d, return_20d, return_60d)
SELECT $1::date, $4 || n, $2, 1.0, 1.0 FROM generate_series(1, $3) AS n
"#;
        let day = |d| NaiveDate::from_ymd_opt(2021, 3, d).expect("日期應合法");
        for (date, return_5d, rows, prefix) in [
            // 最早有 5 日報酬之前、只有長天期的基準日：不列入。
            (day(1), None, 150, "G"),
            (day(2), Some(1.0), 150, "G"),
            // 整天缺 5 日報酬：列出。
            (day(3), None, 120, "G"),
            // 零星缺值（3%）：不列入。
            (day(4), None, 3, "G"),
            (day(4), Some(1.0), 100, "H"),
            // 不常成交的債券 ETF（13%）：不列入。
            (day(5), None, 15, "G"),
            (day(5), Some(1.0), 100, "H"),
            // 一半缺漏：列出缺漏的列數。
            (day(8), None, 60, "G"),
            (day(8), Some(1.0), 60, "H"),
        ] {
            sqlx::query(insert)
                .bind(date)
                .bind(return_5d)
                .bind(rows)
                .bind(prefix)
                .execute(&mut *tx)
                .await
                .expect("寫入應成功");
        }

        let gaps: Vec<(NaiveDate, i64)> = sqlx::query_as(FORWARD_RETURN_GAPS_SQL)
            .bind(FORWARD_RETURN_GAP_MIN_PERCENT)
            .fetch_all(&mut *tx)
            .await
            .expect("查詢應成功");
        tx.rollback().await.expect("rollback 應成功");

        assert_eq!(gaps, [(day(3), 120), (day(8), 60)]);
    }
}
