//! 技術指標日線（[`PriceBarRepository`]）與全市場技術訊號（[`TechnicalSignalRepository`]）
//! 的 PostgreSQL 實作。

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::{Decimal, prelude::ToPrimitive};

use crate::{
    domain::quote::{
        relative_strength::CROSSING_INDICATORS,
        technical::{
            PriceAdjustment, PriceBar, PriceBarRepository, SignalOutcome, StockTechnicalSignal,
            TechnicalSignal, TechnicalSignalQuery, TechnicalSignalRepository, adjust_bars,
        },
    },
    infra::database::{self, repository::quote::PgQuoteRepository},
};

/// 截止日（含）之前最近 `$3` 個有成交的交易日，由新到舊；只讀
/// `(stock_symbol, "Date" DESC)` 覆蓋索引內的欄位（index-only scan）。
const PRICE_BARS_SQL: &str = r#"
SELECT "Date", "ClosingPrice", "HighestPrice", "LowestPrice"
FROM "DailyQuotes"
WHERE stock_symbol = $1
  AND "ClosingPrice" > 0
  AND ($2::date IS NULL OR "Date" <= $2)
ORDER BY "Date" DESC
LIMIT $3
"#;

/// 多檔股票在 `$2`～`$3`（含）有成交的日線，依代號、日期由舊到新；走
/// `(stock_symbol, "Date" DESC)` 覆蓋索引。
const RANGE_BARS_SQL: &str = r#"
SELECT stock_symbol::text, "Date", "ClosingPrice", "HighestPrice", "LowestPrice"
FROM "DailyQuotes"
WHERE stock_symbol = ANY($1)
  AND "Date" BETWEEN $2 AND $3
  AND "ClosingPrice" > 0
ORDER BY stock_symbol, "Date"
"#;

/// 除息（`ex-dividend_date1` 配現金）、除權（`ex-dividend_date2` 配股）與
/// `corporate_action`（減資、分割、反向分割）事件，依股票與日期合併。
///
/// 除權息日欄位是文字，`-` 等非日期值以 CASE 擋掉再轉型。同一天有多列股利時取最大值：
/// 債券 ETF 有月配與季配兩列記同一筆配息的情形，加總會重複計算。
const PRICE_ADJUSTMENTS_SQL: &str = r#"
WITH events AS (
    SELECT security_code AS stock_symbol,
           CASE WHEN "ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$' THEN "ex-dividend_date1"::date END AS date,
           cash_dividend AS cash, 0::numeric AS stock, 1::numeric AS share_ratio
    FROM dividend
    WHERE security_code = ANY($1) AND cash_dividend > 0
    UNION ALL
    SELECT security_code,
           CASE WHEN "ex-dividend_date2" ~ '^\d{4}-\d{2}-\d{2}$' THEN "ex-dividend_date2"::date END,
           0, stock_dividend, 1
    FROM dividend
    WHERE security_code = ANY($1) AND stock_dividend > 0
    UNION ALL
    SELECT stock_symbol, effective_date, 0, 0, share_ratio
    FROM corporate_action
    WHERE stock_symbol = ANY($1) AND share_ratio > 0
)
SELECT stock_symbol, date, MAX(cash), MAX(stock),
       COALESCE(MAX(share_ratio) FILTER (WHERE share_ratio <> 1), 1)
FROM events
WHERE date IS NOT NULL
GROUP BY stock_symbol, date
ORDER BY stock_symbol, date
"#;

#[async_trait]
impl PriceBarRepository for PgQuoteRepository {
    async fn fetch_price_bars(
        &self,
        stock_symbol: &str,
        until: Option<NaiveDate>,
        limit: i64,
    ) -> Result<Vec<PriceBar>> {
        let rows: Vec<(NaiveDate, Decimal, Decimal, Decimal)> = sqlx::query_as(PRICE_BARS_SQL)
            .bind(stock_symbol)
            .bind(until)
            .bind(limit)
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows
            .into_iter()
            .rev()
            .map(|(date, close, high, low)| PriceBar {
                date,
                close: close.to_f64().unwrap_or_default(),
                high: high.to_f64().unwrap_or_default(),
                low: low.to_f64().unwrap_or_default(),
            })
            .collect())
    }

    async fn fetch_price_adjustments(
        &self,
        stock_symbols: &[String],
    ) -> Result<HashMap<String, Vec<PriceAdjustment>>> {
        Ok(self.price_adjustments(stock_symbols).await?)
    }
}

impl PgQuoteRepository {
    /// 多檔股票的除權息與股數變動事件，依代號分組（[`PriceBarRepository::fetch_price_adjustments`]
    /// 的實作，保留 [`sqlx::Error`] 給 Data API 直接轉成錯誤回應）。
    pub(crate) async fn price_adjustments(
        &self,
        stock_symbols: &[String],
    ) -> Result<HashMap<String, Vec<PriceAdjustment>>, sqlx::Error> {
        let rows: Vec<(String, NaiveDate, Decimal, Decimal, Decimal)> =
            sqlx::query_as(PRICE_ADJUSTMENTS_SQL)
                .bind(stock_symbols)
                .fetch_all(database::get_connection())
                .await?;
        let mut out: HashMap<String, Vec<PriceAdjustment>> = HashMap::new();
        for (symbol, date, cash, stock, share_ratio) in rows {
            out.entry(symbol).or_default().push(PriceAdjustment {
                date,
                cash_dividend: cash.to_f64().unwrap_or_default(),
                stock_dividend: stock.to_f64().unwrap_or_default(),
                share_ratio: share_ratio.to_f64().unwrap_or(1.0),
            });
        }
        Ok(out)
    }

    /// 多檔股票在 `from`～`to`（含）有成交的日線，依代號分組、由舊到新，收盤、最高、最低已依期間內的
    /// 除權息與股數變動還原（見 [`adjust_bars`]）；期間內沒有成交的股票不在結果中。回測的期間停損用。
    pub(crate) async fn fetch_adjusted_bars(
        &self,
        stock_symbols: &[String],
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<HashMap<String, Vec<PriceBar>>, sqlx::Error> {
        let rows: Vec<(String, NaiveDate, Decimal, Decimal, Decimal)> =
            sqlx::query_as(RANGE_BARS_SQL)
                .bind(stock_symbols)
                .bind(from)
                .bind(to)
                .fetch_all(database::get_batch_connection())
                .await?;
        let mut bars: HashMap<String, Vec<PriceBar>> = HashMap::new();
        for (symbol, date, close, high, low) in rows {
            bars.entry(symbol).or_default().push(PriceBar {
                date,
                close: close.to_f64().unwrap_or_default(),
                high: high.to_f64().unwrap_or_default(),
                low: low.to_f64().unwrap_or_default(),
            });
        }
        let adjustments = self.price_adjustments(stock_symbols).await?;
        Ok(bars
            .into_iter()
            .map(|(symbol, series)| {
                let events = adjustments.get(&symbol).map_or(&[][..], Vec::as_slice);
                let adjusted = adjust_bars(&series, events);
                (symbol, adjusted)
            })
            .collect())
    }
}

/// 指定交易日有成交、未暫停交易的上市櫃股票。
const SIGNAL_SYMBOLS_SQL: &str = r#"
SELECT s.stock_symbol
FROM stocks s
JOIN "DailyQuotes" q ON q.stock_symbol = s.stock_symbol AND q."Date" = $1
WHERE s."SuspendListing" = FALSE
  AND s.stock_exchange_market_id IN (2, 4)
  AND q."ClosingPrice" > 0
ORDER BY s.stock_symbol
"#;

/// 一次寫入整批訊號（以陣列展開，避免逐筆往返）。
pub(super) const INSERT_SIGNALS_SQL: &str = r#"
INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
SELECT $1, s.stock_symbol, s.indicator, s.kind, s.description
FROM UNNEST($2::varchar[], $3::varchar[], $4::varchar[], $5::text[])
         AS s(stock_symbol, indicator, kind, description)
"#;

#[async_trait]
impl TechnicalSignalRepository for PgQuoteRepository {
    async fn fetch_signal_symbols(&self, date: NaiveDate) -> Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(SIGNAL_SYMBOLS_SQL)
            .bind(date)
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows.into_iter().map(|(symbol,)| symbol).collect())
    }

    async fn replace_signals(
        &self,
        date: NaiveDate,
        signals: &[StockTechnicalSignal],
    ) -> Result<()> {
        let mut tx = database::get_tx().await?;
        // 相對強度穿越訊號由每日相對強度的寫入負責，這裡不動。
        sqlx::query("DELETE FROM daily_technical_signal WHERE date = $1 AND indicator <> ALL($2)")
            .bind(date)
            .bind(&CROSSING_INDICATORS[..])
            .execute(&mut *tx)
            .await?;
        insert_signals(&mut tx, date, signals).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// 在呼叫端的交易內寫入 `date` 的訊號；空清單不送查詢。
pub(super) async fn insert_signals(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    date: NaiveDate,
    signals: &[StockTechnicalSignal],
) -> Result<()> {
    if signals.is_empty() {
        return Ok(());
    }
    let symbols: Vec<&str> = signals.iter().map(|s| s.stock_symbol.as_str()).collect();
    let indicators: Vec<&str> = signals.iter().map(|s| s.signal.indicator).collect();
    let kinds: Vec<&str> = signals.iter().map(|s| s.signal.kind.as_str()).collect();
    let descriptions: Vec<&str> = signals
        .iter()
        .map(|s| s.signal.description.as_str())
        .collect();
    sqlx::query(INSERT_SIGNALS_SQL)
        .bind(date)
        .bind(symbols)
        .bind(indicators)
        .bind(kinds)
        .bind(descriptions)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// 指定交易日、指定股票的訊號。
const SIGNALS_OF_SYMBOLS_SQL: &str = r#"
SELECT stock_symbol, indicator, kind, description
FROM daily_technical_signal
WHERE date = $1 AND stock_symbol = ANY($2)
ORDER BY stock_symbol, indicator, kind
"#;

/// `$1`～`$2` 期間 `$3` 來源的訊號出現後 20 個交易日的還原報酬，依來源與種類統計。
const SIGNAL_OUTCOMES_SQL: &str = r#"
SELECT t.indicator,
       t.kind,
       COUNT(*) AS samples,
       AVG(r.return_20d)::float8 AS avg_return,
       AVG(CASE WHEN r.return_20d > 0 THEN 1.0 ELSE 0.0 END)::float8 AS win_rate
FROM daily_technical_signal t
JOIN daily_forward_return r ON r.date = t.date AND r.stock_symbol = t.stock_symbol
WHERE t.date BETWEEN $1 AND $2
  AND t.indicator = ANY($3)
  AND r.return_20d IS NOT NULL
GROUP BY t.indicator, t.kind
ORDER BY t.indicator, t.kind
"#;

#[async_trait]
impl TechnicalSignalQuery for PgQuoteRepository {
    async fn fetch_signals(
        &self,
        date: NaiveDate,
        stock_symbols: &[String],
    ) -> Result<Vec<StockTechnicalSignal>> {
        let rows: Vec<(String, String, String, String)> = sqlx::query_as(SIGNALS_OF_SYMBOLS_SQL)
            .bind(date)
            .bind(stock_symbols)
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(stock_symbol, indicator, kind, description)| {
                Some(StockTechnicalSignal {
                    date,
                    stock_symbol,
                    signal: TechnicalSignal::from_names(&indicator, &kind, description)?,
                })
            })
            .collect())
    }

    async fn fetch_signal_outcomes(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        indicators: &[&str],
    ) -> Result<Vec<SignalOutcome>> {
        let indicators: Vec<String> = indicators.iter().map(|s| (*s).to_owned()).collect();
        let rows: Vec<(String, String, i64, f64, f64)> = sqlx::query_as(SIGNAL_OUTCOMES_SQL)
            .bind(from)
            .bind(to)
            .bind(&indicators)
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows
            .into_iter()
            .map(
                |(indicator, kind, samples, avg_return, win_rate)| SignalOutcome {
                    indicator,
                    kind,
                    samples,
                    avg_return,
                    win_rate,
                },
            )
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::quote::technical::{SignalKind, TechnicalSignal};

    fn signal(symbol: &str, kind: SignalKind) -> StockTechnicalSignal {
        StockTechnicalSignal {
            date: NaiveDate::from_ymd_opt(1991, 9, 2).expect("有效日期"),
            stock_symbol: symbol.to_owned(),
            signal: TechnicalSignal {
                indicator: "kd",
                kind,
                description: format!("{symbol} 測試訊號"),
            },
        }
    }

    /// 只列出當日有成交、未暫停的上市櫃股票；寫入會整日覆蓋舊訊號。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn signal_symbols_and_replace_round_trip() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 signal_symbols_and_replace_round_trip：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        let date = NaiveDate::from_ymd_opt(1991, 9, 2).expect("有效日期");
        let cleanup = || async {
            for sql in [
                r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979S_'"#,
                "DELETE FROM stocks WHERE stock_symbol LIKE '79979S_'",
                "DELETE FROM daily_technical_signal WHERE date = '1991-09-02'",
            ] {
                sqlx::query(sql).execute(pool).await.expect("清理測試資料");
            }
        };
        cleanup().await;

        // A 上市有成交、B 上櫃但收盤價 0、C 暫停交易、D 市場別 0。
        for (symbol, market, suspended, close) in [
            ("79979SA", 2, false, 10),
            ("79979SB", 4, false, 0),
            ("79979SC", 2, true, 10),
            ("79979SD", 0, false, 10),
        ] {
            sqlx::query(
                r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, "SuspendListing", stock_exchange_market_id, stock_industry_id)
                   VALUES ($1, $1, $1, $2, $3, 0)"#,
            )
            .bind(symbol)
            .bind(suspended)
            .bind(market)
            .execute(pool)
            .await
            .expect("寫入測試股票");
            sqlx::query(
                r#"INSERT INTO "DailyQuotes" (stock_symbol, "Date", "ClosingPrice", year, month, day)
                   VALUES ($1, $2, $3, 1991, 9, 2)"#,
            )
            .bind(symbol)
            .bind(date)
            .bind(rust_decimal::Decimal::from(close))
            .execute(pool)
            .await
            .expect("寫入測試報價");
        }

        let repo = PgQuoteRepository::new();
        let symbols = repo.fetch_signal_symbols(date).await.expect("查詢代號");
        let ours: Vec<&String> = symbols.iter().filter(|s| s.starts_with("79979S")).collect();
        assert_eq!(ours, ["79979SA"]);

        let first = [
            signal("79979SA", SignalKind::GoldenCross),
            signal("79979SA", SignalKind::Overbought),
        ];
        repo.replace_signals(date, &first)
            .await
            .expect("第一次寫入");
        let read = repo
            .fetch_signals(date, &["79979SA".to_owned(), "79979SB".to_owned()])
            .await
            .expect("讀回訊號");
        assert_eq!(read, first);
        sqlx::query(
            "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
             VALUES ($1, '79979SA', 'unknown', 'golden_cross', '舊資料')",
        )
        .bind(date)
        .execute(pool)
        .await
        .expect("寫入無法辨識的訊號");
        assert_eq!(
            repo.fetch_signals(date, &["79979SA".to_owned()])
                .await
                .expect("讀回訊號")
                .len(),
            2,
            "無法辨識的來源應略過"
        );
        // 相對強度穿越訊號由每日相對強度負責寫入，整天替換技術訊號時要保留。
        sqlx::query(
            "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
             VALUES ($1, '79979SA', 'rs80', 'cross_above', '相對強度 76 → 82 分，突破 80 分')",
        )
        .bind(date)
        .execute(pool)
        .await
        .expect("寫入相對強度訊號");
        repo.replace_signals(date, &first[1..])
            .await
            .expect("覆蓋寫入");
        let rows: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT stock_symbol, indicator, kind, description FROM daily_technical_signal
             WHERE date = $1 ORDER BY indicator",
        )
        .bind(date)
        .fetch_all(pool)
        .await
        .expect("讀回訊號");
        assert_eq!(
            rows,
            [
                (
                    "79979SA".to_owned(),
                    "kd".to_owned(),
                    "overbought".to_owned(),
                    "79979SA 測試訊號".to_owned()
                ),
                (
                    "79979SA".to_owned(),
                    "rs80".to_owned(),
                    "cross_above".to_owned(),
                    "相對強度 76 → 82 分，突破 80 分".to_owned()
                )
            ]
        );

        repo.replace_signals(date, &[]).await.expect("清空");
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM daily_technical_signal WHERE date = $1")
                .bind(date)
                .fetch_one(pool)
                .await
                .expect("計數");
        assert_eq!(count, 1, "只剩相對強度訊號");

        cleanup().await;
    }

    /// 除息、除權與公司行動依股票與日期合併；同日重複的股利取最大值，非日期的除權息日略過。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn price_adjustments_merge_dividends_and_corporate_actions() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 price_adjustments_merge_dividends_and_corporate_actions：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        let cleanup = || async {
            for sql in [
                "DELETE FROM dividend WHERE security_code LIKE '79979J_'",
                "DELETE FROM corporate_action WHERE stock_symbol LIKE '79979J_'",
            ] {
                sqlx::query(sql).execute(pool).await.expect("清理測試資料");
            }
        };
        cleanup().await;

        // JA：同日月配與季配重複（取一筆）、另一年同日除權息、日期為「-」的年度彙總列。
        for (symbol, year, quarter, cash, stock, date1, date2) in [
            ("79979JA", 1991, "M04", "0.14", "0", "1991-07-16", "-"),
            ("79979JA", 1991, "Q2", "0.14", "0", "1991-07-16", "-"),
            (
                "79979JA",
                1992,
                "",
                "1.5",
                "0.5",
                "1992-08-01",
                "1992-08-01",
            ),
            ("79979JA", 1993, "", "2", "0", "-", "-"),
            ("79979JB", 1991, "", "0", "1", "-", "1991-09-02"),
        ] {
            sqlx::query(
                r#"INSERT INTO dividend (security_code, year, quarter, cash_dividend, stock_dividend,
                                         "ex-dividend_date1", "ex-dividend_date2")
                   VALUES ($1, $2, $3, $4::numeric, $5::numeric, $6, $7)"#,
            )
            .bind(symbol)
            .bind(year)
            .bind(quarter)
            .bind(cash)
            .bind(stock)
            .bind(date1)
            .bind(date2)
            .execute(pool)
            .await
            .expect("寫入測試股利");
        }
        sqlx::query(
            "INSERT INTO corporate_action (stock_symbol, effective_date, action_type, share_ratio)
             VALUES ('79979JB', '1991-10-01', 'capital_reduction', 0.7)",
        )
        .execute(pool)
        .await
        .expect("寫入測試公司行動");

        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).expect("有效日期");
        let map = PgQuoteRepository::new()
            .fetch_price_adjustments(&[
                "79979JA".to_owned(),
                "79979JB".to_owned(),
                "79979JC".to_owned(),
            ])
            .await
            .expect("查詢事件");
        cleanup().await;

        let adjustment = |date, cash, stock, share_ratio| PriceAdjustment {
            date,
            cash_dividend: cash,
            stock_dividend: stock,
            share_ratio,
        };
        assert_eq!(map.len(), 2, "{map:?}");
        assert_eq!(
            map["79979JA"],
            [
                adjustment(day(1991, 7, 16), 0.14, 0.0, 1.0),
                adjustment(day(1992, 8, 1), 1.5, 0.5, 1.0),
            ]
        );
        assert_eq!(
            map["79979JB"],
            [
                adjustment(day(1991, 9, 2), 0.0, 1.0, 1.0),
                adjustment(day(1991, 10, 1), 0.0, 0.0, 0.7),
            ]
        );
    }

    /// 只統計指定來源、已滿 20 日的訊號；依來源與種類分組。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn signal_outcomes_group_by_indicator_and_kind() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 signal_outcomes_group_by_indicator_and_kind：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        let day = |d| NaiveDate::from_ymd_opt(1989, 3, d).expect("有效日期");
        let cleanup = || async {
            for sql in [
                "DELETE FROM daily_technical_signal WHERE stock_symbol LIKE '79979O_'",
                "DELETE FROM daily_forward_return WHERE stock_symbol LIKE '79979O_'",
            ] {
                sqlx::query(sql).execute(pool).await.expect("清理測試資料");
            }
        };
        cleanup().await;

        // OA、OB 黃金交叉 +10%、−2%；OC 死亡交叉未滿 20 日；OD 是其他來源；OE 在期間外。
        for (d, symbol, indicator, kind, value) in [
            (3, "79979OA", "kd", "golden_cross", Some(10)),
            (3, "79979OB", "kd", "golden_cross", Some(-2)),
            (3, "79979OC", "kd", "death_cross", None),
            (3, "79979OD", "rsi14", "overbought", Some(5)),
            (28, "79979OE", "kd", "golden_cross", Some(50)),
        ] {
            sqlx::query(
                "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
                 VALUES ($1, $2, $3, $4, '測試')",
            )
            .bind(day(d))
            .bind(symbol)
            .bind(indicator)
            .bind(kind)
            .execute(pool)
            .await
            .expect("寫入測試訊號");
            sqlx::query(
                "INSERT INTO daily_forward_return (date, stock_symbol, return_20d) VALUES ($1, $2, $3)",
            )
            .bind(day(d))
            .bind(symbol)
            .bind(value.map(rust_decimal::Decimal::from))
            .execute(pool)
            .await
            .expect("寫入測試報酬");
        }

        let outcomes = PgQuoteRepository::new()
            .fetch_signal_outcomes(day(1), day(20), &["kd"])
            .await
            .expect("統計訊號表現");
        cleanup().await;
        assert_eq!(
            outcomes,
            [SignalOutcome {
                indicator: "kd".to_owned(),
                kind: "golden_cross".to_owned(),
                samples: 2,
                avg_return: 4.0,
                win_rate: 0.5,
            }]
        );
    }
}
