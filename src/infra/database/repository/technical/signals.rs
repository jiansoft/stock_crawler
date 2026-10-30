//! 全市場技術訊號的寫入與查詢，以及訊號之後的報酬統計。

use super::*;

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
const INSERT_SIGNALS_SQL: &str = r#"
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

    async fn fetch_traded_symbols(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(TRADED_SYMBOLS_SQL)
            .bind(from)
            .bind(to)
            .fetch_all(database::get_batch_connection())
            .await?;
        Ok(rows.into_iter().map(|(symbol,)| symbol).collect())
    }

    async fn replace_indicator_signals(
        &self,
        indicator: &str,
        from: NaiveDate,
        to: NaiveDate,
        signals: &[StockTechnicalSignal],
    ) -> Result<()> {
        let mut tx = database::get_tx().await?;
        sqlx::query(
            "DELETE FROM daily_technical_signal WHERE indicator = $1 AND date BETWEEN $2 AND $3",
        )
        .bind(indicator)
        .bind(from)
        .bind(to)
        .execute(&mut *tx)
        .await?;
        if !signals.is_empty() {
            let dates: Vec<NaiveDate> = signals.iter().map(|s| s.date).collect();
            let symbols: Vec<&str> = signals.iter().map(|s| s.stock_symbol.as_str()).collect();
            let indicators: Vec<&str> = signals.iter().map(|s| s.signal.indicator).collect();
            let kinds: Vec<&str> = signals.iter().map(|s| s.signal.kind.as_str()).collect();
            let descriptions: Vec<&str> = signals
                .iter()
                .map(|s| s.signal.description.as_str())
                .collect();
            sqlx::query(INSERT_DATED_SIGNALS_SQL)
                .bind(dates)
                .bind(symbols)
                .bind(indicators)
                .bind(kinds)
                .bind(descriptions)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

/// `$1`～`$2`（含）期間有成交的上市櫃股票，含之後下市的。
const TRADED_SYMBOLS_SQL: &str = r#"
SELECT DISTINCT q.stock_symbol::text
FROM "DailyQuotes" q
JOIN stocks s ON s.stock_symbol = q.stock_symbol
WHERE q."Date" BETWEEN $1 AND $2
  AND q."ClosingPrice" > 0
  AND s.stock_exchange_market_id IN (2, 4)
ORDER BY 1
"#;

/// 一次寫入跨多個交易日的訊號（歷史回補用）。
const INSERT_DATED_SIGNALS_SQL: &str = r#"
INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
SELECT s.date, s.stock_symbol, s.indicator, s.kind, s.description
FROM UNNEST($1::date[], $2::varchar[], $3::varchar[], $4::varchar[], $5::text[])
         AS s(date, stock_symbol, indicator, kind, description)
"#;

/// 在呼叫端的交易內寫入 `date` 的訊號；空清單不送查詢。
pub(in crate::infra::database::repository) async fn insert_signals(
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

/// `$1` 交易日 `$2` 來源的訊號，依當日成交值由大到小取前 `$3` 檔，附當天該來源的總檔數。
const RANKED_SIGNALS_SQL: &str = r#"
SELECT t.stock_symbol::text,
       COALESCE(s."Name", '')::text AS name,
       COALESCE(si.name, '')::text AS industry,
       COALESCE(q."TradeValue", 0)::float8 AS trade_value,
       t.description,
       COUNT(*) OVER () AS total
FROM daily_technical_signal t
LEFT JOIN stocks s ON s.stock_symbol = t.stock_symbol
LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
LEFT JOIN "DailyQuotes" q ON q.stock_symbol = t.stock_symbol AND q."Date" = t.date
WHERE t.date = $1 AND t.indicator = $2 AND t.kind = $3
ORDER BY trade_value DESC, t.stock_symbol
LIMIT $4
"#;

#[async_trait]
impl RankedSignalQuery for PgQuoteRepository {
    async fn fetch_ranked_signals(
        &self,
        date: NaiveDate,
        indicator: &str,
        kind: SignalKind,
        limit: i64,
    ) -> Result<(Vec<RankedSignal>, i64)> {
        let rows: Vec<(String, String, String, f64, String, i64)> =
            sqlx::query_as(RANKED_SIGNALS_SQL)
                .bind(date)
                .bind(indicator)
                .bind(kind.as_str())
                .bind(limit)
                .fetch_all(database::get_connection())
                .await?;
        let total = rows.first().map_or(0, |row| row.5);
        let signals = rows
            .into_iter()
            .map(
                |(stock_symbol, name, industry, trade_value, description, _)| RankedSignal {
                    stock_symbol,
                    name,
                    industry,
                    trade_value,
                    description,
                },
            )
            .collect();
        Ok((signals, total))
    }
}
