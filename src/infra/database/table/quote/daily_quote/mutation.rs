//! `DailyQuote` 的資料庫寫入／更新操作。
//!
//! 包含單筆 upsert 與使用 `COPY` 的批次寫入；均線與年內統計的回填、重算與批次更新在
//! `moving_average`。

use anyhow::{Context, Result};
use sqlx::postgres::PgQueryResult;

use crate::infra::database;

use super::{COPY_IN_QUERY, DailyQuote};

impl DailyQuote {
    /// 將當前報價寫入資料庫，若主鍵衝突則更新既有資料。
    ///
    /// 「upsert」= update + insert：SQL 中的 `ON CONFLICT ("stock_symbol", "Date")
    /// DO UPDATE` 表示——若該股票在該交易日還沒有資料就 INSERT 新增一筆；
    /// 若已存在（撞到唯一索引）則改用 UPDATE 以新值覆蓋價格相關欄位。
    /// 適合單筆補寫；大量寫入請改用 [`Self::copy_in_raw`] 系列以求效能。
    pub async fn upsert(&self) -> Result<PgQueryResult> {
        let sql = r#"
       INSERT INTO "DailyQuotes" (
            maximum_price_in_year_date_on,
            minimum_price_in_year_date_on,
            "Date",
            "CreateTime",
            "RecordTime",
            "PriceEarningRatio",
            "MovingAverage60",
            "ClosingPrice",
            "ChangeRange",
            "Change",
            "LastBestBidPrice",
            "LastBestBidVolume",
            "LastBestAskPrice",
            "LastBestAskVolume",
            "MovingAverage5",
            "MovingAverage10",
            "MovingAverage20",
            "LowestPrice",
            "MovingAverage120",
            "MovingAverage240",
            maximum_price_in_year,
            minimum_price_in_year,
            average_price_in_year,
            "HighestPrice",
            "OpeningPrice",
            "TradingVolume",
            "TradeValue",
            "Transaction",
            "price-to-book_ratio",
            "stock_symbol",
            year,
            month,
            day
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33)
        ON CONFLICT ("stock_symbol", "Date")
        DO UPDATE SET
            "RecordTime" = now(),
            "ClosingPrice" = excluded."ClosingPrice",
            "ChangeRange" = excluded."ChangeRange",
            "Change" = excluded. "Change",
            "LastBestBidPrice" = excluded. "LastBestBidPrice",
            "LastBestBidVolume" = excluded."LastBestBidVolume",
            "LastBestAskPrice" = excluded."LastBestAskPrice",
            "LastBestAskVolume" = excluded."LastBestAskVolume",
            "LowestPrice" = excluded."LowestPrice",
            "HighestPrice" = excluded."HighestPrice",
            "OpeningPrice" = excluded."OpeningPrice",
            "TradingVolume" = excluded."TradingVolume",
            "TradeValue" = excluded."TradeValue",
            "Transaction" = excluded."Transaction",
            "price-to-book_ratio" = excluded."price-to-book_ratio",
            "PriceEarningRatio" = excluded."PriceEarningRatio"
    "#;
        sqlx::query(sql)
            .bind(self.maximum_price_in_year_date_on)
            .bind(self.minimum_price_in_year_date_on)
            .bind(self.date)
            .bind(self.create_time)
            .bind(self.record_time)
            .bind(self.price_earning_ratio)
            .bind(self.moving_average_60)
            .bind(self.closing_price)
            .bind(self.change_range)
            .bind(self.change)
            .bind(self.last_best_bid_price)
            .bind(self.last_best_bid_volume)
            .bind(self.last_best_ask_price)
            .bind(self.last_best_ask_volume)
            .bind(self.moving_average_5)
            .bind(self.moving_average_10)
            .bind(self.moving_average_20)
            .bind(self.lowest_price)
            .bind(self.moving_average_120)
            .bind(self.moving_average_240)
            .bind(self.maximum_price_in_year)
            .bind(self.minimum_price_in_year)
            .bind(self.average_price_in_year)
            .bind(self.highest_price)
            .bind(self.opening_price)
            .bind(self.trading_volume)
            .bind(self.trade_value)
            .bind(self.transaction)
            .bind(self.price_to_book_ratio)
            .bind(&self.stock_symbol)
            .bind(self.year)
            .bind(self.month)
            .bind(self.day)
            .execute(database::get_connection())
            .await
            .context(format!(
                "Failed to DailyQuote::upsert({:#?}) from database",
                self
            ))
    }

    /// 使用 `COPY` 批次寫入 `DailyQuotes`。
    ///
    /// 這個版本會自行從連線池取連線，屬於獨立的自動提交操作；
    /// 若需要與其他 SQL（例如先刪除同日資料）綁在同一個 transaction 內，
    /// 請改用 [`Self::copy_in_raw_on`]。
    pub async fn copy_in_raw(quotes: &[Self]) -> Result<u64> {
        database::copy_in_raw(COPY_IN_QUERY, quotes).await
    }

    /// 在「呼叫端指定的連線」上使用 `COPY` 批次寫入 `DailyQuotes`。
    ///
    /// 呼叫端把 transaction 的連線（`&mut *tx`）傳進來時，這批寫入就會
    /// 跟同一個 transaction 內的其他 SQL 一起 commit 或 rollback。
    /// 「日報價原子替換」（先刪後寫、同生共死）正是靠這個函式達成：
    /// 中途任何一步失敗，資料庫都會回到 transaction 開始前的狀態。
    pub async fn copy_in_raw_on(conn: &mut sqlx::PgConnection, quotes: &[Self]) -> Result<u64> {
        database::copy_in_raw_on(conn, COPY_IN_QUERY, quotes).await
    }

    /// 批次補寫缺漏的日報價，已存在者原封不動，回傳實際新增的列數。
    ///
    /// 專供回補歷史缺口使用，因此是 `ON CONFLICT DO NOTHING` 而非 upsert：
    /// 補洞不該覆寫既有資料 —— 來源（`STOCK_DAY`）沒有本益比、股價淨值比與
    /// 最佳買賣揭示，若用 `DO UPDATE` 會把既有列的那些欄位清成 0。
    /// `COPY` 在這裡也不能用：它不支援 `ON CONFLICT`，區間內只要有一天已存在
    /// 就會整批因唯一索引而失敗。
    pub async fn insert_missing_batch(quotes: &[Self]) -> Result<u64> {
        if quotes.is_empty() {
            return Ok(0);
        }

        let mut builder = sqlx::QueryBuilder::new(
            r#"INSERT INTO "DailyQuotes" (
                "Date", stock_symbol, year, month, day,
                "OpeningPrice", "HighestPrice", "LowestPrice", "ClosingPrice",
                "Change", "ChangeRange", "TradingVolume", "TradeValue", "Transaction",
                maximum_price_in_year_date_on, minimum_price_in_year_date_on
            ) "#,
        );

        builder.push_values(quotes, |mut row, quote| {
            row.push_bind(quote.date)
                .push_bind(quote.stock_symbol.clone())
                .push_bind(quote.year)
                .push_bind(quote.month)
                .push_bind(quote.day)
                .push_bind(quote.opening_price)
                .push_bind(quote.highest_price)
                .push_bind(quote.lowest_price)
                .push_bind(quote.closing_price)
                .push_bind(quote.change)
                .push_bind(quote.change_range)
                .push_bind(quote.trading_volume)
                .push_bind(quote.trade_value)
                .push_bind(quote.transaction)
                .push_bind(quote.maximum_price_in_year_date_on)
                .push_bind(quote.minimum_price_in_year_date_on);
        });
        builder.push(r#" ON CONFLICT (stock_symbol, "Date") DO NOTHING"#);

        let result = builder
            .build()
            .execute(database::get_connection())
            .await
            .context("Failed to insert missing daily quotes")?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests;
