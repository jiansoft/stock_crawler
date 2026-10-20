//! CAGR 倉儲的寫入：批次 upsert 與刪除舊資料。

use anyhow::{Context, Result};
use chrono::NaiveDate;

use crate::domain::performance::entity::StockCagr as DomainStockCagr;
use crate::infra::database;
use crate::infra::database::table::performance::stock_cagr::StockCagr as TableStockCagr;

/// 單批寫入的最大列數。
///
/// PostgreSQL 單一敘述最多 65535 個參數，本表以 26 個陣列參數承載任意列數，
/// 參數個數不受列數影響；限制批量的目的是控制單次網路封包與伺服器端的
/// 記憶體用量（全市場 × 8 期間約 1.8 萬列）。
const SAVE_BATCH_SIZE: usize = 2_000;

/// 批次 upsert 指定基準日的 CAGR 計算結果。
///
/// 刻意不使用 `COPY`：本專案的 [`CopyIn`](crate::infra::database) 無法處理
/// `ON CONFLICT`，同日重跑會整批因主鍵重複而失敗。改以 `UNNEST` 展開
/// 26 個陣列參數做多列插入，兼顧「一次往返寫入數千列」與 upsert 語意。
pub(super) async fn save_batch(records: &[DomainStockCagr]) -> Result<u64> {
    if records.is_empty() {
        return Ok(0);
    }

    let sql = r#"
INSERT INTO stock_cagr (
date, stock_symbol, period, base_date, base_price, end_price, years,
price_end_shares, price_end_value, price_return_pct, price_cagr_pct,
total_shares, total_cash, total_end_value, total_return_pct, total_cagr_pct,
reinv_shares, reinv_cash, reinv_end_value, reinv_return_pct, reinv_cagr_pct,
first_quote_date, shortfall_days, data_complete, has_anomaly, dividend_events
)
SELECT * FROM UNNEST(
$1::date[], $2::varchar[], $3::varchar[], $4::date[], $5::numeric[], $6::numeric[], $7::numeric[],
$8::numeric[], $9::numeric[], $10::numeric[], $11::numeric[],
$12::numeric[], $13::numeric[], $14::numeric[], $15::numeric[], $16::numeric[],
$17::numeric[], $18::numeric[], $19::numeric[], $20::numeric[], $21::numeric[],
$22::date[], $23::int4[], $24::bool[], $25::bool[], $26::int4[]
) AS t(
date, stock_symbol, period, base_date, base_price, end_price, years,
price_end_shares, price_end_value, price_return_pct, price_cagr_pct,
total_shares, total_cash, total_end_value, total_return_pct, total_cagr_pct,
reinv_shares, reinv_cash, reinv_end_value, reinv_return_pct, reinv_cagr_pct,
first_quote_date, shortfall_days, data_complete, has_anomaly, dividend_events
)
ON CONFLICT (date, stock_symbol, period) DO UPDATE SET
base_date = EXCLUDED.base_date,
base_price = EXCLUDED.base_price,
end_price = EXCLUDED.end_price,
years = EXCLUDED.years,
price_end_shares = EXCLUDED.price_end_shares,
price_end_value = EXCLUDED.price_end_value,
price_return_pct = EXCLUDED.price_return_pct,
price_cagr_pct = EXCLUDED.price_cagr_pct,
total_shares = EXCLUDED.total_shares,
total_cash = EXCLUDED.total_cash,
total_end_value = EXCLUDED.total_end_value,
total_return_pct = EXCLUDED.total_return_pct,
total_cagr_pct = EXCLUDED.total_cagr_pct,
reinv_shares = EXCLUDED.reinv_shares,
reinv_cash = EXCLUDED.reinv_cash,
reinv_end_value = EXCLUDED.reinv_end_value,
reinv_return_pct = EXCLUDED.reinv_return_pct,
reinv_cagr_pct = EXCLUDED.reinv_cagr_pct,
first_quote_date = EXCLUDED.first_quote_date,
shortfall_days = EXCLUDED.shortfall_days,
data_complete = EXCLUDED.data_complete,
has_anomaly = EXCLUDED.has_anomaly,
dividend_events = EXCLUDED.dividend_events,
updated_time = now();
"#;

    let mut affected: u64 = 0;

    for chunk in records.chunks(SAVE_BATCH_SIZE) {
        let len = chunk.len();
        let mut dates = Vec::with_capacity(len);
        let mut symbols = Vec::with_capacity(len);
        let mut periods = Vec::with_capacity(len);
        let mut base_dates = Vec::with_capacity(len);
        let mut base_prices = Vec::with_capacity(len);
        let mut end_prices = Vec::with_capacity(len);
        let mut years = Vec::with_capacity(len);
        let mut price_end_shares = Vec::with_capacity(len);
        let mut price_end_values = Vec::with_capacity(len);
        let mut price_return_pcts = Vec::with_capacity(len);
        let mut price_cagr_pcts = Vec::with_capacity(len);
        let mut total_shares = Vec::with_capacity(len);
        let mut total_cashes = Vec::with_capacity(len);
        let mut total_end_values = Vec::with_capacity(len);
        let mut total_return_pcts = Vec::with_capacity(len);
        let mut total_cagr_pcts = Vec::with_capacity(len);
        let mut reinv_shares = Vec::with_capacity(len);
        let mut reinv_cashes = Vec::with_capacity(len);
        let mut reinv_end_values = Vec::with_capacity(len);
        let mut reinv_return_pcts = Vec::with_capacity(len);
        let mut reinv_cagr_pcts = Vec::with_capacity(len);
        let mut first_quote_dates = Vec::with_capacity(len);
        let mut shortfall_days = Vec::with_capacity(len);
        let mut data_completes = Vec::with_capacity(len);
        let mut has_anomalies = Vec::with_capacity(len);
        let mut dividend_events = Vec::with_capacity(len);

        for record in chunk {
            let row = TableStockCagr::from(record);
            dates.push(row.date);
            symbols.push(row.stock_symbol);
            periods.push(row.period);
            base_dates.push(row.base_date);
            base_prices.push(row.base_price);
            end_prices.push(row.end_price);
            years.push(row.years);
            price_end_shares.push(row.price_end_shares);
            price_end_values.push(row.price_end_value);
            price_return_pcts.push(row.price_return_pct);
            price_cagr_pcts.push(row.price_cagr_pct);
            total_shares.push(row.total_shares);
            total_cashes.push(row.total_cash);
            total_end_values.push(row.total_end_value);
            total_return_pcts.push(row.total_return_pct);
            total_cagr_pcts.push(row.total_cagr_pct);
            reinv_shares.push(row.reinv_shares);
            reinv_cashes.push(row.reinv_cash);
            reinv_end_values.push(row.reinv_end_value);
            reinv_return_pcts.push(row.reinv_return_pct);
            reinv_cagr_pcts.push(row.reinv_cagr_pct);
            first_quote_dates.push(row.first_quote_date);
            shortfall_days.push(row.shortfall_days);
            data_completes.push(row.data_complete);
            has_anomalies.push(row.has_anomaly);
            dividend_events.push(row.dividend_events);
        }

        let result = sqlx::query(sql)
            .bind(&dates)
            .bind(&symbols)
            .bind(&periods)
            .bind(&base_dates)
            .bind(&base_prices)
            .bind(&end_prices)
            .bind(&years)
            .bind(&price_end_shares)
            .bind(&price_end_values)
            .bind(&price_return_pcts)
            .bind(&price_cagr_pcts)
            .bind(&total_shares)
            .bind(&total_cashes)
            .bind(&total_end_values)
            .bind(&total_return_pcts)
            .bind(&total_cagr_pcts)
            .bind(&reinv_shares)
            .bind(&reinv_cashes)
            .bind(&reinv_end_values)
            .bind(&reinv_return_pcts)
            .bind(&reinv_cagr_pcts)
            .bind(&first_quote_dates)
            .bind(&shortfall_days)
            .bind(&data_completes)
            .bind(&has_anomalies)
            .bind(&dividend_events)
            .execute(database::get_connection())
            .await
            .context("Failed to upsert stock_cagr in PgCagrRepository::save_batch")?;

        affected += result.rows_affected();
    }

    Ok(affected)
}

/// 刪除早於指定日期的歷史資料。
pub(super) async fn delete_before(date: NaiveDate) -> Result<u64> {
    let result = sqlx::query("DELETE FROM stock_cagr WHERE date < $1")
        .bind(date)
        .execute(database::get_connection())
        .await
        .context("Failed to delete stock_cagr in PgCagrRepository::delete_before")?;

    Ok(result.rows_affected())
}
