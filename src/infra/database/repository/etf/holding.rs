//! ETF 前十大持股與配置：最新一期（`etf_holding`、`etf_allocation`）、每期歷史
//! （`etf_holding_history`）與最新兩期的比對來源。

use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::{
    domain::etf::{
        EtfAllocationRecord, EtfHoldingPeriod, EtfHoldingPeriods, EtfHoldingRecord, EtfPortfolio,
    },
    infra::database,
};

/// 指定 ETF 最新兩期的前十大持股；只取最新一期在 `$2`（含）之後才收到、且有前一期的。
const HOLDING_PERIODS_SQL: &str = r#"
WITH periods AS (
    SELECT stock_symbol, as_of, MIN(recorded_at) AS recorded_at,
           ROW_NUMBER() OVER (PARTITION BY stock_symbol ORDER BY as_of DESC) AS rn
    FROM etf_holding_history
    WHERE stock_symbol = ANY($1)
    GROUP BY stock_symbol, as_of
),
fresh AS (
    SELECT p.stock_symbol FROM periods AS p
    WHERE p.rn = 1 AND p.recorded_at >= $2::date
      AND EXISTS (SELECT 1 FROM periods AS q WHERE q.stock_symbol = p.stock_symbol AND q.rn = 2)
)
SELECT h.stock_symbol::text AS stock_symbol, p.rn, h.as_of, h.rank, h.holding_symbol::text AS holding_symbol,
       h.holding_name, h.weight
FROM etf_holding_history AS h
JOIN periods AS p ON p.stock_symbol = h.stock_symbol AND p.as_of = h.as_of
JOIN fresh AS f ON f.stock_symbol = h.stock_symbol
WHERE p.rn <= 2
ORDER BY h.stock_symbol, p.rn, h.rank
"#;

async fn replace_allocation(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    symbol: &str,
    kind: &str,
    as_of: NaiveDate,
    rows: &[EtfAllocationRecord],
) -> Result<u64> {
    sqlx::query("DELETE FROM etf_allocation WHERE stock_symbol = $1 AND kind = $2")
        .bind(symbol)
        .bind(kind)
        .execute(&mut **tx)
        .await?;
    let mut written = 0;
    // 同名的列（來源偶有重複）只留第一筆，避免主鍵衝突。
    let mut seen = std::collections::HashSet::new();
    for (index, row) in rows.iter().enumerate() {
        if !seen.insert(row.label.as_str()) {
            continue;
        }
        written += sqlx::query(
            "INSERT INTO etf_allocation (stock_symbol, kind, label, as_of, weight, sort_order)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(symbol)
        .bind(kind)
        .bind(&row.label)
        .bind(as_of)
        .bind(row.weight)
        .bind(i16::try_from(index).unwrap_or(i16::MAX))
        .execute(&mut **tx)
        .await
        .with_context(|| format!("Failed to save ETF {kind} allocation {symbol}"))?
        .rows_affected();
    }
    Ok(written)
}

pub(super) async fn replace_portfolio(portfolio: &EtfPortfolio) -> Result<u64> {
    let symbol = portfolio.stock_symbol.as_str();
    let mut tx = database::get_tx().await?;
    let mut written = 0;

    if let (Some(as_of), false) = (portfolio.holdings_as_of, portfolio.holdings.is_empty()) {
        sqlx::query("DELETE FROM etf_holding WHERE stock_symbol = $1")
            .bind(symbol)
            .execute(&mut *tx)
            .await?;
        for h in &portfolio.holdings {
            written += sqlx::query(
                "INSERT INTO etf_holding (stock_symbol, rank, as_of, holding_symbol, holding_name, weight)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(symbol)
            .bind(h.rank)
            .bind(as_of)
            .bind(&h.holding_symbol)
            .bind(&h.holding_name)
            .bind(h.weight)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("Failed to save ETF holdings {symbol}"))?
            .rows_affected();
            // 每一期另存一份；同一期重抓只更正內容，保留第一次收到的時間。
            sqlx::query(
                "INSERT INTO etf_holding_history (stock_symbol, as_of, rank, holding_symbol, holding_name, weight)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (stock_symbol, as_of, rank) DO UPDATE SET
                     holding_symbol = EXCLUDED.holding_symbol, holding_name = EXCLUDED.holding_name,
                     weight = EXCLUDED.weight",
            )
            .bind(symbol)
            .bind(as_of)
            .bind(h.rank)
            .bind(&h.holding_symbol)
            .bind(&h.holding_name)
            .bind(h.weight)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("Failed to save ETF holding history {symbol}"))?;
        }
    }

    for (kind, as_of, rows) in [
        (
            "industry",
            portfolio.industries_as_of,
            &portfolio.industries,
        ),
        ("asset", portfolio.assets_as_of, &portfolio.assets),
    ] {
        let Some(as_of) = as_of.filter(|_| !rows.is_empty()) else {
            continue;
        };
        written += replace_allocation(&mut tx, symbol, kind, as_of, rows).await?;
    }
    tx.commit().await?;
    crate::infra::cache::etf_analysis::ETF_ANALYSIS.invalidate();
    Ok(written)
}

pub(super) async fn holding_periods(
    stock_symbols: &[String],
    recorded_since: NaiveDate,
) -> Result<Vec<EtfHoldingPeriods>> {
    let rows: Vec<(String, i64, NaiveDate, i16, String, String, Decimal)> =
        sqlx::query_as(HOLDING_PERIODS_SQL)
            .bind(stock_symbols)
            .bind(recorded_since)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch ETF holding periods")?;
    let mut out: Vec<EtfHoldingPeriods> = Vec::new();
    for (symbol, rn, as_of, rank, holding_symbol, holding_name, weight) in rows {
        if out.last().is_none_or(|last| last.stock_symbol != symbol) {
            let empty = EtfHoldingPeriod {
                as_of,
                holdings: Vec::new(),
            };
            out.push(EtfHoldingPeriods {
                stock_symbol: symbol.clone(),
                previous: empty.clone(),
                current: empty,
            });
        }
        let entry = out.last_mut().expect("剛放入");
        let period = if rn == 1 {
            &mut entry.current
        } else {
            &mut entry.previous
        };
        period.as_of = as_of;
        period.holdings.push(EtfHoldingRecord {
            rank,
            holding_symbol,
            holding_name,
            weight,
        });
    }
    Ok(out)
}
