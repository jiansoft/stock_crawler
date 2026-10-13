//! 技術指標日線（[`PriceBarRepository`]）與全市場技術訊號（[`TechnicalSignalRepository`]）
//! 的 PostgreSQL 實作。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::{Decimal, prelude::ToPrimitive};

use crate::{
    domain::quote::technical::{
        PriceBar, PriceBarRepository, StockTechnicalSignal, TechnicalSignalRepository,
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
        sqlx::query("DELETE FROM daily_technical_signal WHERE date = $1")
            .bind(date)
            .execute(&mut *tx)
            .await?;
        if !signals.is_empty() {
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
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
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
        repo.replace_signals(date, &first[1..])
            .await
            .expect("覆蓋寫入");
        let rows: Vec<(String, String, String, String)> = sqlx::query_as(
            "SELECT stock_symbol, indicator, kind, description FROM daily_technical_signal WHERE date = $1",
        )
        .bind(date)
        .fetch_all(pool)
        .await
        .expect("讀回訊號");
        assert_eq!(
            rows,
            [(
                "79979SA".to_owned(),
                "kd".to_owned(),
                "overbought".to_owned(),
                "79979SA 測試訊號".to_owned()
            )]
        );

        repo.replace_signals(date, &[]).await.expect("清空");
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM daily_technical_signal WHERE date = $1")
                .bind(date)
                .fetch_one(pool)
                .await
                .expect("計數");
        assert_eq!(count, 0);

        cleanup().await;
    }
}
