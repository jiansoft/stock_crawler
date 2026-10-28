//! `holder_distribution`（集保股權分散週摘要）、`holder_distribution_level`（全部持股分級）
//! 與 `insider_holding`（董監持股與設質）。

use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::dedup_by_key;
use crate::{
    domain::chip::{
        HolderDistributionRecord, HolderLevelRecord, InsiderHoldingRecord, RecentHolderWeeks,
    },
    infra::database,
};

/// `as_of`（含）以前最近的幾個資料週；主鍵以日期開頭，倒序掃索引取到足夠的週數就停。
const RECENT_WEEKS_SQL: &str = r#"
SELECT DISTINCT "date" FROM holder_distribution
WHERE "date" <= $1
ORDER BY "date" DESC
LIMIT $2
"#;

const RECENT_RECORDS_SQL: &str = r#"
SELECT "date", stock_symbol, major_holders, major_percent, total_holders,
       holders_400, shares_400, percent_400
FROM holder_distribution
WHERE stock_symbol = ANY($1) AND "date" = ANY($2)
ORDER BY stock_symbol, "date" DESC
"#;

type HolderRow = (
    NaiveDate,
    String,
    i64,
    Decimal,
    i64,
    Option<i64>,
    Option<i64>,
    Option<Decimal>,
);

pub(super) async fn recent_holder_weeks(
    symbols: &[String],
    as_of: NaiveDate,
    weeks: i64,
) -> Result<RecentHolderWeeks> {
    let dates: Vec<NaiveDate> = sqlx::query_scalar(RECENT_WEEKS_SQL)
        .bind(as_of)
        .bind(weeks)
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch recent holder distribution weeks")?;
    if dates.is_empty() || symbols.is_empty() {
        return Ok(RecentHolderWeeks {
            dates,
            records: Vec::new(),
        });
    }
    let rows: Vec<HolderRow> = sqlx::query_as(RECENT_RECORDS_SQL)
        .bind(symbols)
        .bind(&dates)
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch recent holder distributions")?;
    let records = rows
        .into_iter()
        .map(
            |(
                date,
                stock_symbol,
                major_holders,
                major_percent,
                total_holders,
                holders_400,
                shares_400,
                percent_400,
            )| HolderDistributionRecord {
                date,
                stock_symbol,
                major_holders,
                major_percent,
                total_holders,
                holders_400,
                shares_400,
                percent_400,
            },
        )
        .collect();
    Ok(RecentHolderWeeks { dates, records })
}

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
    let holders_400: Vec<Option<i64>> = records.iter().map(|r| r.holders_400).collect();
    let shares_400: Vec<Option<i64>> = records.iter().map(|r| r.shares_400).collect();
    let percent_400: Vec<Option<Decimal>> = records.iter().map(|r| r.percent_400).collect();

    let sql = r#"
INSERT INTO holder_distribution ("date", stock_symbol, major_holders, major_percent, total_holders,
    holders_400, shares_400, percent_400)
SELECT * FROM UNNEST($1::date[], $2::varchar[], $3::bigint[], $4::numeric[], $5::bigint[],
    $6::bigint[], $7::bigint[], $8::numeric[])
ON CONFLICT ("date", stock_symbol) DO UPDATE SET
major_holders = EXCLUDED.major_holders,
major_percent = EXCLUDED.major_percent,
total_holders = EXCLUDED.total_holders,
holders_400 = EXCLUDED.holders_400,
shares_400 = EXCLUDED.shares_400,
percent_400 = EXCLUDED.percent_400,
updated_time = now()
WHERE (holder_distribution.major_holders, holder_distribution.major_percent, holder_distribution.total_holders,
       holder_distribution.holders_400, holder_distribution.shares_400, holder_distribution.percent_400)
  IS DISTINCT FROM (EXCLUDED.major_holders, EXCLUDED.major_percent, EXCLUDED.total_holders,
       EXCLUDED.holders_400, EXCLUDED.shares_400, EXCLUDED.percent_400)
"#;
    let result = sqlx::query(sql)
        .bind(&dates)
        .bind(&symbols)
        .bind(&major_holders)
        .bind(&major_percent)
        .bind(&total_holders)
        .bind(&holders_400)
        .bind(&shares_400)
        .bind(&percent_400)
        .execute(database::get_connection())
        .await
        .context("Failed to save holder distributions")?;
    Ok(result.rows_affected())
}

pub(super) async fn save_holder_levels(records: &[HolderLevelRecord]) -> Result<u64> {
    let records = dedup_by_key(records, |r| (r.date, r.stock_symbol.clone(), r.level));
    if records.is_empty() {
        return Ok(0);
    }
    let dates: Vec<NaiveDate> = records.iter().map(|r| r.date).collect();
    let symbols: Vec<&str> = records.iter().map(|r| r.stock_symbol.as_str()).collect();
    let levels: Vec<i16> = records.iter().map(|r| r.level).collect();
    let holders: Vec<i64> = records.iter().map(|r| r.holders).collect();
    let shares: Vec<i64> = records.iter().map(|r| r.shares).collect();
    let percent: Vec<Decimal> = records.iter().map(|r| r.percent).collect();

    let sql = r#"
INSERT INTO holder_distribution_level ("date", stock_symbol, level, holders, shares, percent)
SELECT * FROM UNNEST($1::date[], $2::varchar[], $3::smallint[], $4::bigint[], $5::bigint[], $6::numeric[])
ON CONFLICT ("date", stock_symbol, level) DO UPDATE SET
holders = EXCLUDED.holders,
shares = EXCLUDED.shares,
percent = EXCLUDED.percent,
updated_time = now()
WHERE (holder_distribution_level.holders, holder_distribution_level.shares, holder_distribution_level.percent)
  IS DISTINCT FROM (EXCLUDED.holders, EXCLUDED.shares, EXCLUDED.percent)
"#;
    let result = sqlx::query(sql)
        .bind(&dates)
        .bind(&symbols)
        .bind(&levels)
        .bind(&holders)
        .bind(&shares)
        .bind(&percent)
        .execute(database::get_connection())
        .await
        .context("Failed to save holder distribution levels")?;
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
