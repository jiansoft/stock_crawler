//! `chip_daily`：三大法人買賣超、融資融券與連續買賣超天數。

use anyhow::{Context, Result};
use chrono::NaiveDate;

use super::dedup_by_key;
use crate::{
    domain::chip::{InstitutionalNet, MarginRecord},
    infra::database,
};

/// 重算 `$1`～`$2` 每列的連續買賣超天數（外資、投信、自營商、三者合計），只更新有變的列。
///
/// - `base`／`grouped`：方向（淨額正負號，缺值當 0）依「依日期的列號 − 依方向分組的列號」切出
///   連續段；同一檔期間內第一段的分組值必為 0。
/// - `counted`：每列在所屬連續段內是第幾天。
/// - `seed`：每檔 `$1` 之前最後一列的天數（走 `(stock_symbol, date DESC)` 索引）；第一段方向
///   相同時接續累加，因此逐日或分段重算的結果與一次重算全期相同。
const REFRESH_STREAKS_SQL: &str = r#"
WITH base AS (
    SELECT stock_symbol, "date",
           SIGN(COALESCE(foreign_net, 0))::int AS fs,
           SIGN(COALESCE(trust_net, 0))::int AS ts,
           SIGN(COALESCE(dealer_net, 0))::int AS ds,
           SIGN(COALESCE(foreign_net, 0) + COALESCE(trust_net, 0) + COALESCE(dealer_net, 0))::int AS os,
           ROW_NUMBER() OVER (PARTITION BY stock_symbol ORDER BY "date") AS rn
    FROM chip_daily
    WHERE "date" BETWEEN $1 AND $2
),
grouped AS (
    SELECT stock_symbol, "date", fs, ts, ds, os,
           rn - ROW_NUMBER() OVER (PARTITION BY stock_symbol, fs ORDER BY "date") AS fg,
           rn - ROW_NUMBER() OVER (PARTITION BY stock_symbol, ts ORDER BY "date") AS tg,
           rn - ROW_NUMBER() OVER (PARTITION BY stock_symbol, ds ORDER BY "date") AS dg,
           rn - ROW_NUMBER() OVER (PARTITION BY stock_symbol, os ORDER BY "date") AS og
    FROM base
),
counted AS (
    SELECT stock_symbol, "date", fs, ts, ds, os, fg, tg, dg, og,
           ROW_NUMBER() OVER (PARTITION BY stock_symbol, fs, fg ORDER BY "date") AS fl,
           ROW_NUMBER() OVER (PARTITION BY stock_symbol, ts, tg ORDER BY "date") AS tl,
           ROW_NUMBER() OVER (PARTITION BY stock_symbol, ds, dg ORDER BY "date") AS dl,
           ROW_NUMBER() OVER (PARTITION BY stock_symbol, os, og ORDER BY "date") AS ol
    FROM grouped
),
seed AS (
    SELECT sym.stock_symbol,
           COALESCE(p.foreign_streak, 0) AS fp,
           COALESCE(p.trust_streak, 0) AS tp,
           COALESCE(p.dealer_streak, 0) AS dp,
           COALESCE(p.total_streak, 0) AS op
    FROM (SELECT DISTINCT stock_symbol FROM base) AS sym
    CROSS JOIN LATERAL (
        SELECT c.foreign_streak, c.trust_streak, c.dealer_streak, c.total_streak
        FROM chip_daily c
        WHERE c.stock_symbol = sym.stock_symbol AND c."date" < $1
        ORDER BY c."date" DESC
        LIMIT 1
    ) AS p
),
computed AS (
    SELECT c.stock_symbol, c."date",
           (c.fs * (c.fl + CASE WHEN c.fg = 0 AND SIGN(COALESCE(s.fp, 0)) = c.fs
                                THEN ABS(COALESCE(s.fp, 0)) ELSE 0 END))::smallint AS foreign_streak,
           (c.ts * (c.tl + CASE WHEN c.tg = 0 AND SIGN(COALESCE(s.tp, 0)) = c.ts
                                THEN ABS(COALESCE(s.tp, 0)) ELSE 0 END))::smallint AS trust_streak,
           (c.ds * (c.dl + CASE WHEN c.dg = 0 AND SIGN(COALESCE(s.dp, 0)) = c.ds
                                THEN ABS(COALESCE(s.dp, 0)) ELSE 0 END))::smallint AS dealer_streak,
           (c.os * (c.ol + CASE WHEN c.og = 0 AND SIGN(COALESCE(s.op, 0)) = c.os
                                THEN ABS(COALESCE(s.op, 0)) ELSE 0 END))::smallint AS total_streak
    FROM counted c
    LEFT JOIN seed s ON s.stock_symbol = c.stock_symbol
)
UPDATE chip_daily d
SET foreign_streak = x.foreign_streak,
    trust_streak = x.trust_streak,
    dealer_streak = x.dealer_streak,
    total_streak = x.total_streak
FROM computed x
WHERE d."date" = x."date" AND d.stock_symbol = x.stock_symbol
  AND (d.foreign_streak, d.trust_streak, d.dealer_streak, d.total_streak)
      IS DISTINCT FROM (x.foreign_streak, x.trust_streak, x.dealer_streak, x.total_streak)
"#;

pub(super) async fn save_institutional(date: NaiveDate, flows: &[InstitutionalNet]) -> Result<u64> {
    let flows = dedup_by_key(flows, |flow| flow.stock_symbol.clone());
    if flows.is_empty() {
        return Ok(0);
    }
    let symbols: Vec<&str> = flows.iter().map(|f| f.stock_symbol.as_str()).collect();
    let foreign: Vec<i64> = flows.iter().map(|f| f.foreign).collect();
    let trust: Vec<i64> = flows.iter().map(|f| f.trust).collect();
    let dealer: Vec<i64> = flows.iter().map(|f| f.dealer).collect();

    let sql = r#"
INSERT INTO chip_daily ("date", stock_symbol, foreign_net, trust_net, dealer_net)
SELECT $1, t.symbol, t.foreign_net, t.trust_net, t.dealer_net
FROM UNNEST($2::varchar[], $3::bigint[], $4::bigint[], $5::bigint[])
 AS t(symbol, foreign_net, trust_net, dealer_net)
ON CONFLICT ("date", stock_symbol) DO UPDATE SET
foreign_net = EXCLUDED.foreign_net,
trust_net = EXCLUDED.trust_net,
dealer_net = EXCLUDED.dealer_net,
updated_time = now()
WHERE (chip_daily.foreign_net, chip_daily.trust_net, chip_daily.dealer_net)
  IS DISTINCT FROM (EXCLUDED.foreign_net, EXCLUDED.trust_net, EXCLUDED.dealer_net)
"#;
    let result = sqlx::query(sql)
        .bind(date)
        .bind(&symbols)
        .bind(&foreign)
        .bind(&trust)
        .bind(&dealer)
        .execute(database::get_connection())
        .await
        .with_context(|| format!("Failed to save institutional flows of {date}"))?;
    Ok(result.rows_affected())
}

pub(super) async fn save_margin(date: NaiveDate, margins: &[MarginRecord]) -> Result<u64> {
    let margins = dedup_by_key(margins, |margin| margin.stock_symbol.clone());
    if margins.is_empty() {
        return Ok(0);
    }
    let symbols: Vec<&str> = margins.iter().map(|m| m.stock_symbol.as_str()).collect();
    let margin_previous: Vec<i64> = margins.iter().map(|m| m.margin_previous).collect();
    let margin_balance: Vec<i64> = margins.iter().map(|m| m.margin_balance).collect();
    let short_previous: Vec<i64> = margins.iter().map(|m| m.short_previous).collect();
    let short_balance: Vec<i64> = margins.iter().map(|m| m.short_balance).collect();

    let sql = r#"
INSERT INTO chip_daily ("date", stock_symbol, margin_previous, margin_balance, short_previous, short_balance)
SELECT $1, t.symbol, t.margin_previous, t.margin_balance, t.short_previous, t.short_balance
FROM UNNEST($2::varchar[], $3::bigint[], $4::bigint[], $5::bigint[], $6::bigint[])
 AS t(symbol, margin_previous, margin_balance, short_previous, short_balance)
ON CONFLICT ("date", stock_symbol) DO UPDATE SET
margin_previous = EXCLUDED.margin_previous,
margin_balance = EXCLUDED.margin_balance,
short_previous = EXCLUDED.short_previous,
short_balance = EXCLUDED.short_balance,
updated_time = now()
WHERE (chip_daily.margin_previous, chip_daily.margin_balance, chip_daily.short_previous, chip_daily.short_balance)
  IS DISTINCT FROM (EXCLUDED.margin_previous, EXCLUDED.margin_balance, EXCLUDED.short_previous, EXCLUDED.short_balance)
"#;
    let result = sqlx::query(sql)
        .bind(date)
        .bind(&symbols)
        .bind(&margin_previous)
        .bind(&margin_balance)
        .bind(&short_previous)
        .bind(&short_balance)
        .execute(database::get_connection())
        .await
        .with_context(|| format!("Failed to save margin balances of {date}"))?;
    Ok(result.rows_affected())
}

pub(super) async fn refresh_streaks(from: NaiveDate, to: NaiveDate) -> Result<u64> {
    let result = sqlx::query(REFRESH_STREAKS_SQL)
        .bind(from)
        .bind(to)
        .execute(database::get_connection())
        .await
        .with_context(|| format!("Failed to refresh chip streaks of {from}~{to}"))?;
    Ok(result.rows_affected())
}
