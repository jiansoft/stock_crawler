//! `holder_distribution`（集保股權分散）與 `insider_holding`（董監持股與設質）。

use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::dedup_by_key;
use crate::{
    domain::chip::{HolderDistributionRecord, InsiderHoldingRecord},
    infra::database,
};

pub(super) async fn save_holder_distributions(records: &[HolderDistributionRecord]) -> Result<u64> {
    let records = dedup_by_key(records, |r| (r.date, r.stock_symbol.clone()));
    if records.is_empty() {
        return Ok(0);
    }
    let dates: Vec<NaiveDate> = records.iter().map(|r| r.date).collect();
    let symbols: Vec<&str> = records.iter().map(|r| r.stock_symbol.as_str()).collect();
    let major_holders: Vec<i64> = records.iter().map(|r| r.major_holders).collect();
    let major_percent: Vec<Decimal> = records.iter().map(|r| r.major_percent).collect();
    let total_holders: Vec<i64> = records.iter().map(|r| r.total_holders).collect();

    let sql = r#"
INSERT INTO holder_distribution ("date", stock_symbol, major_holders, major_percent, total_holders)
SELECT * FROM UNNEST($1::date[], $2::varchar[], $3::bigint[], $4::numeric[], $5::bigint[])
ON CONFLICT ("date", stock_symbol) DO UPDATE SET
major_holders = EXCLUDED.major_holders,
major_percent = EXCLUDED.major_percent,
total_holders = EXCLUDED.total_holders,
updated_time = now()
WHERE (holder_distribution.major_holders, holder_distribution.major_percent, holder_distribution.total_holders)
  IS DISTINCT FROM (EXCLUDED.major_holders, EXCLUDED.major_percent, EXCLUDED.total_holders)
"#;
    let result = sqlx::query(sql)
        .bind(&dates)
        .bind(&symbols)
        .bind(&major_holders)
        .bind(&major_percent)
        .bind(&total_holders)
        .execute(database::get_connection())
        .await
        .context("Failed to save holder distributions")?;
    Ok(result.rows_affected())
}

pub(super) async fn replace_insider_holdings(records: &[InsiderHoldingRecord]) -> Result<u64> {
    let records = dedup_by_key(records, |r| {
        (
            r.month,
            r.stock_symbol.clone(),
            r.title.clone(),
            r.name.clone(),
        )
    });
    if records.is_empty() {
        return Ok(0);
    }
    let months: Vec<NaiveDate> = records.iter().map(|r| r.month).collect();
    let symbols: Vec<&str> = records.iter().map(|r| r.stock_symbol.as_str()).collect();
    let titles: Vec<&str> = records.iter().map(|r| r.title.as_str()).collect();
    let names: Vec<&str> = records.iter().map(|r| r.name.as_str()).collect();
    let shares: Vec<i64> = records.iter().map(|r| r.shares).collect();
    let pledged: Vec<i64> = records.iter().map(|r| r.pledged).collect();
    let related: Vec<i64> = records.iter().map(|r| r.related_pledged).collect();

    // 同月份、同股票的舊名單先刪掉不在這批裡的人（辭任、改派），再寫入這批。
    let delete = r#"
DELETE FROM insider_holding AS h
USING (SELECT DISTINCT * FROM UNNEST($1::date[], $2::varchar[]) AS t("month", stock_symbol)) AS batch
WHERE h."month" = batch."month" AND h.stock_symbol = batch.stock_symbol
  AND (h."month", h.stock_symbol, h.title, h.name) NOT IN (
  SELECT * FROM UNNEST($1::date[], $2::varchar[], $3::varchar[], $4::varchar[])
  )
"#;
    let upsert = r#"
INSERT INTO insider_holding ("month", stock_symbol, title, name, shares, pledged, related_pledged)
SELECT * FROM UNNEST($1::date[], $2::varchar[], $3::varchar[], $4::varchar[], $5::bigint[], $6::bigint[], $7::bigint[])
ON CONFLICT ("month", stock_symbol, title, name) DO UPDATE SET
shares = EXCLUDED.shares,
pledged = EXCLUDED.pledged,
related_pledged = EXCLUDED.related_pledged,
updated_time = now()
WHERE (insider_holding.shares, insider_holding.pledged, insider_holding.related_pledged)
  IS DISTINCT FROM (EXCLUDED.shares, EXCLUDED.pledged, EXCLUDED.related_pledged)
"#;
    let mut tx = database::get_tx().await?;
    let removed = sqlx::query(delete)
        .bind(&months)
        .bind(&symbols)
        .bind(&titles)
        .bind(&names)
        .execute(&mut *tx)
        .await
        .context("Failed to remove stale insider holdings")?
        .rows_affected();
    let written = sqlx::query(upsert)
        .bind(&months)
        .bind(&symbols)
        .bind(&titles)
        .bind(&names)
        .bind(&shares)
        .bind(&pledged)
        .bind(&related)
        .execute(&mut *tx)
        .await
        .context("Failed to save insider holdings")?
        .rows_affected();
    tx.commit()
        .await
        .context("Failed to commit insider holdings")?;
    Ok(removed + written)
}
