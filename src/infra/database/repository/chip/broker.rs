//! `broker_flow`：主力分點進出。

use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::dedup_by_key;
use crate::{domain::chip::BrokerFlowRecord, infra::database};

pub(super) async fn save_broker_flows(records: &[BrokerFlowRecord]) -> Result<u64> {
    let records = dedup_by_key(records, |r| (r.date, r.stock_symbol.clone()));
    if records.is_empty() {
        return Ok(0);
    }
    let dates: Vec<NaiveDate> = records.iter().map(|r| r.date).collect();
    let symbols: Vec<&str> = records.iter().map(|r| r.stock_symbol.as_str()).collect();
    let buy_totals: Vec<i64> = records.iter().map(|r| r.buy_total).collect();
    let sell_totals: Vec<i64> = records.iter().map(|r| r.sell_total).collect();
    let main_shares: Vec<Decimal> = records.iter().map(|r| r.main_share).collect();
    let buyers = records
        .iter()
        .map(|r| serde_json::to_string(&r.buyers))
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to serialize broker buyers")?;
    let sellers = records
        .iter()
        .map(|r| serde_json::to_string(&r.sellers))
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to serialize broker sellers")?;

    let sql = r#"
INSERT INTO broker_flow ("date", stock_symbol, buy_total, sell_total, main_share, buyers, sellers)
SELECT t.d, t.symbol, t.buy_total, t.sell_total, t.main_share, t.buyers::jsonb, t.sellers::jsonb
FROM UNNEST($1::date[], $2::varchar[], $3::bigint[], $4::bigint[], $5::numeric[], $6::text[], $7::text[])
 AS t(d, symbol, buy_total, sell_total, main_share, buyers, sellers)
ON CONFLICT ("date", stock_symbol) DO UPDATE SET
buy_total = EXCLUDED.buy_total,
sell_total = EXCLUDED.sell_total,
main_share = EXCLUDED.main_share,
buyers = EXCLUDED.buyers,
sellers = EXCLUDED.sellers,
updated_time = now()
WHERE (broker_flow.buy_total, broker_flow.sell_total, broker_flow.main_share, broker_flow.buyers, broker_flow.sellers)
  IS DISTINCT FROM (EXCLUDED.buy_total, EXCLUDED.sell_total, EXCLUDED.main_share, EXCLUDED.buyers, EXCLUDED.sellers)
"#;
    let result = sqlx::query(sql)
        .bind(&dates)
        .bind(&symbols)
        .bind(&buy_totals)
        .bind(&sell_totals)
        .bind(&main_shares)
        .bind(&buyers)
        .bind(&sellers)
        .execute(database::get_connection())
        .await
        .context("Failed to save broker flows")?;
    Ok(result.rows_affected())
}
