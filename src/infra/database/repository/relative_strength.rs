//! 相對強度（全體計分、每日分數的讀寫）的 PostgreSQL 實作。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::{
    domain::quote::{
        forward_return::ForwardReturnRepository,
        relative_strength::{
            CROSSING_INDICATORS, LONGEST, PeriodCrossing, RelativeStrengthCrossingQuery,
            RelativeStrengthDay, RelativeStrengthQuery, RelativeStrengthRepository, ScoredStock,
            ScoredUniverseQuery, UniverseRow, base_dates, parse_crossing, score_universe,
        },
        technical::StockTechnicalSignal,
    },
    infra::database::{
        self,
        repository::{quote::PgQuoteRepository, technical::insert_signals},
    },
};

/// 資料日 `$1` 的計分母體：上市櫃、近 5 個交易日內有收盤、不是 ETF／ETN／受益證券（產業 9001～9003）
/// 與特別股（代號 4 碼數字後接英文字母，例如 2887Z1），且近 60／120／240 日報酬都有的股票。
///
/// `$2`～`$5` 是往前第 20、60、120、240 個交易日（基準日）；近 N 日報酬＝該基準日的 N 日報酬。
/// `$6` 是往前第 5 個交易日：用「當時有沒有在交易」判斷，而不是「現在有沒有下市」，回補歷史時
/// 後來才下市的股票也在當天的母體裡，沒有存活者偏差；下市或停止交易超過 5 個交易日就不再計分。
const UNIVERSE_SQL: &str = r#"
SELECT
    r60.stock_symbol,
    s."Name" AS name,
    s.stock_exchange_market_id AS market_id,
    COALESCE(si.name, '') AS industry,
    r20.return_20d::float8 AS return_20d,
    r60.return_60d::float8 AS return_60d,
    r120.return_120d::float8 AS return_120d,
    r240.return_240d::float8 AS return_240d,
    q."ClosingPrice"::float8 AS closing_price
FROM daily_forward_return r60
JOIN daily_forward_return r120 ON r120.date = $4 AND r120.stock_symbol = r60.stock_symbol
JOIN daily_forward_return r240 ON r240.date = $5 AND r240.stock_symbol = r60.stock_symbol
LEFT JOIN daily_forward_return r20 ON r20.date = $2 AND r20.stock_symbol = r60.stock_symbol
JOIN stocks s ON s.stock_symbol = r60.stock_symbol
LEFT JOIN stock_industry si ON si.stock_industry_id = s.stock_industry_id
LEFT JOIN "DailyQuotes" q
       ON q.stock_symbol = r60.stock_symbol AND q."Date" = $1 AND q."ClosingPrice" > 0
WHERE r60.date = $3
  AND r60.return_60d IS NOT NULL
  AND r120.return_120d IS NOT NULL
  AND r240.return_240d IS NOT NULL
  AND s.stock_exchange_market_id IN (2, 4)
  AND s.stock_industry_id NOT IN (9001, 9002, 9003)
  AND s.stock_symbol !~ '^[0-9]{4}[A-Z]'
  AND EXISTS (
      SELECT 1 FROM "DailyQuotes" a
      WHERE a.stock_symbol = r60.stock_symbol
        AND a."Date" > $6 AND a."Date" <= $1
        AND a."ClosingPrice" > 0
  )
"#;

/// `$1`（含）之前最近 `$2` 個已寫入相對強度的交易日，取股票 `$3` 的分數與名次。
///
/// 只回看 400 個日曆天，足以涵蓋 120 個交易日。
const HISTORY_SQL: &str = r#"
WITH td AS (
    SELECT DISTINCT date
    FROM daily_relative_strength
    WHERE date <= $1 AND date > $1 - 400
    ORDER BY date DESC
    LIMIT $2
)
SELECT r.stock_symbol, r.date, r.weighted_return::float8 AS weighted, r.rs_score, r.rank, r.universe
FROM daily_relative_strength r
JOIN td ON td.date = r.date
WHERE r.stock_symbol = ANY($3)
ORDER BY r.stock_symbol, r.date DESC
"#;

/// 整天替換：刪除 `$1` 的舊分數。
const DELETE_DAY_SQL: &str = "DELETE FROM daily_relative_strength WHERE date = $1";

/// 整天替換：刪除 `$1` 的相對強度穿越訊號（來源 `$2`），其他技術訊號不動。
const DELETE_DAY_SIGNALS_SQL: &str =
    "DELETE FROM daily_technical_signal WHERE date = $1 AND indicator = ANY($2)";

/// 整天替換：寫入 `$1` 的分數（代號、加權報酬、分數、名次四個平行陣列，`$6` 為母體數）。
const INSERT_DAY_SQL: &str = r#"
INSERT INTO daily_relative_strength (date, stock_symbol, weighted_return, rs_score, rank, universe)
SELECT $1, symbol, weighted, score, rank, $6
FROM UNNEST($2::text[], $3::float8[], $4::smallint[], $5::int[]) AS t(symbol, weighted, score, rank)
"#;

/// `$1`～`$2` 每檔股票對每個門檻（來源 `$3`）的最後一次穿越，附 `$2` 當天的分數。
const PERIOD_CROSSINGS_SQL: &str = r#"
SELECT DISTINCT ON (t.stock_symbol, t.indicator)
       t.stock_symbol, t.indicator, t.kind, rs.rs_score
FROM daily_technical_signal t
LEFT JOIN daily_relative_strength rs ON rs.date = $2 AND rs.stock_symbol = t.stock_symbol
WHERE t.date BETWEEN $1 AND $2
  AND t.indicator = ANY($3)
ORDER BY t.stock_symbol, t.indicator, t.date DESC
"#;

/// [`UNIVERSE_SQL`] 的一列。
#[derive(sqlx::FromRow)]
struct UniverseSqlRow {
    stock_symbol: String,
    name: String,
    market_id: i32,
    industry: String,
    return_20d: Option<f64>,
    return_60d: f64,
    return_120d: f64,
    return_240d: f64,
    closing_price: Option<f64>,
}

/// [`HISTORY_SQL`] 的一列。
#[derive(sqlx::FromRow)]
struct HistoryRow {
    stock_symbol: String,
    date: NaiveDate,
    weighted: f64,
    rs_score: i16,
    rank: i32,
    universe: i32,
}

#[async_trait]
impl RelativeStrengthQuery for PgQuoteRepository {
    async fn fetch_relative_strength(
        &self,
        until: NaiveDate,
        days: u16,
        stock_symbols: &[String],
    ) -> Result<Vec<RelativeStrengthDay>> {
        let rows: Vec<HistoryRow> = sqlx::query_as(HISTORY_SQL)
            .bind(until)
            .bind(i32::from(days))
            .bind(stock_symbols)
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| RelativeStrengthDay {
                stock_symbol: row.stock_symbol,
                date: row.date,
                weighted: row.weighted,
                rs_score: u8::try_from(row.rs_score).unwrap_or_default(),
                rank: i64::from(row.rank),
                universe: i64::from(row.universe),
            })
            .collect())
    }
}

#[async_trait]
impl RelativeStrengthRepository for PgQuoteRepository {
    async fn replace_relative_strength(
        &self,
        date: NaiveDate,
        scored: &[ScoredStock],
        signals: &[StockTechnicalSignal],
    ) -> Result<u64> {
        let symbols: Vec<&str> = scored
            .iter()
            .map(|stock| stock.row.stock_symbol.as_str())
            .collect();
        let weighted: Vec<f64> = scored.iter().map(|stock| stock.weighted).collect();
        let scores: Vec<i16> = scored
            .iter()
            .map(|stock| i16::from(stock.rs_score))
            .collect();
        let ranks: Vec<i32> = scored
            .iter()
            .map(|stock| i32::try_from(stock.rank).unwrap_or(i32::MAX))
            .collect();
        let universe = i32::try_from(scored.len()).unwrap_or(i32::MAX);
        let mut tx = database::get_tx().await?;
        sqlx::query(DELETE_DAY_SQL)
            .bind(date)
            .execute(&mut *tx)
            .await?;
        let written = sqlx::query(INSERT_DAY_SQL)
            .bind(date)
            .bind(&symbols)
            .bind(&weighted)
            .bind(&scores)
            .bind(&ranks)
            .bind(universe)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        sqlx::query(DELETE_DAY_SIGNALS_SQL)
            .bind(date)
            .bind(&CROSSING_INDICATORS[..])
            .execute(&mut *tx)
            .await?;
        insert_signals(&mut tx, date, signals).await?;
        tx.commit().await?;
        Ok(written)
    }
}

#[async_trait]
impl RelativeStrengthCrossingQuery for PgQuoteRepository {
    async fn fetch_period_crossings(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<PeriodCrossing>> {
        let rows: Vec<(String, String, String, Option<i16>)> = sqlx::query_as(PERIOD_CROSSINGS_SQL)
            .bind(from)
            .bind(to)
            .bind(&CROSSING_INDICATORS[..])
            .fetch_all(database::get_connection())
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(stock_symbol, indicator, kind, score)| {
                let (threshold, direction) = parse_crossing(&indicator, &kind)?;
                Some(PeriodCrossing {
                    stock_symbol,
                    threshold,
                    direction,
                    latest_score: score.and_then(|score| u8::try_from(score).ok()),
                })
            })
            .collect())
    }
}

#[async_trait]
impl ScoredUniverseQuery for PgQuoteRepository {
    async fn fetch_scored_universe(
        &self,
        until: NaiveDate,
    ) -> Result<Option<(NaiveDate, Vec<ScoredStock>)>> {
        let dates = self.fetch_trading_dates(until, LONGEST + 2).await?;
        for offset in 0..2 {
            let Some(bases) = dates.get(offset..).and_then(base_dates) else {
                break;
            };
            let rows: Vec<UniverseSqlRow> = sqlx::query_as(UNIVERSE_SQL)
                .bind(bases.end)
                .bind(bases.d20)
                .bind(bases.d60)
                .bind(bases.d120)
                .bind(bases.d240)
                .bind(bases.active_since)
                .fetch_all(database::get_connection())
                .await?;
            if !rows.is_empty() {
                let rows = rows
                    .into_iter()
                    .map(|row| UniverseRow {
                        stock_symbol: row.stock_symbol,
                        name: row.name,
                        market_id: row.market_id,
                        industry: row.industry,
                        return_20d: row.return_20d,
                        return_60d: row.return_60d,
                        return_120d: row.return_120d,
                        return_240d: row.return_240d,
                        closing_price: row.closing_price,
                    })
                    .collect();
                return Ok(Some((bases.end, score_universe(rows))));
            }
        }
        Ok(None)
    }
}

/// 排行榜與回測共用的 SQL 排序：分數、全母體名次、代號。
macro_rules! rs_rank_order {
    () => {
        "rs_score DESC, rs_rank ASC, stock_symbol"
    };
}
pub(crate) use rs_rank_order;

/// 每日留存的排名加上同日歷史報酬明細，避免 API 即時計分與回測母體分歧。
#[derive(sqlx::FromRow)]
struct StoredUniverseRow {
    #[sqlx(flatten)]
    detail: UniverseSqlRow,
    rs_rank: i32,
    rs_score: i16,
    stored_weighted: f64,
}

impl PgQuoteRepository {
    /// 查詢已留存的每日 RS 母體，排序與回測完全共用；未計算的日期向前找。
    pub(crate) async fn fetch_stored_scored_universe(
        &self,
        until: NaiveDate,
    ) -> Result<Option<(NaiveDate, Vec<ScoredStock>)>> {
        let date: Option<NaiveDate> =
            sqlx::query_scalar("SELECT MAX(date) FROM daily_relative_strength WHERE date <= $1")
                .bind(until)
                .fetch_one(database::get_connection())
                .await?;
        let Some(date) = date else { return Ok(None) };
        let dates = self.fetch_trading_dates(date, LONGEST + 2).await?;
        let Some(bases) = base_dates(&dates) else {
            return Ok(None);
        };
        let sql = format!(
            "WITH details AS ({UNIVERSE_SQL}) SELECT d.*, r.rank AS rs_rank, r.rs_score, r.weighted_return::float8 AS stored_weighted FROM daily_relative_strength r JOIN details d USING(stock_symbol) WHERE r.date=$1 ORDER BY {}",
            rs_rank_order!()
        );
        // SQL 僅由靜態查詢與共用排序組合；所有外部值仍以 bind 傳入。
        let records: Vec<StoredUniverseRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(date)
            .bind(bases.d20)
            .bind(bases.d60)
            .bind(bases.d120)
            .bind(bases.d240)
            .bind(bases.active_since)
            .fetch_all(database::get_connection())
            .await?;
        // 不重算或捏造缺少歷史明細的排名；資料不完整時回查無資料。
        let expected: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM daily_relative_strength WHERE date=$1")
                .bind(date)
                .fetch_one(database::get_connection())
                .await?;
        if records.len() as i64 != expected || records.is_empty() {
            return Ok(None);
        }
        let scored = records
            .into_iter()
            .map(|record| {
                let row = record.detail;
                ScoredStock {
                    rank: record.rs_rank as usize,
                    rs_score: record.rs_score as u8,
                    weighted: record.stored_weighted,
                    row: UniverseRow {
                        stock_symbol: row.stock_symbol,
                        name: row.name,
                        market_id: row.market_id,
                        industry: row.industry,
                        return_20d: row.return_20d,
                        return_60d: row.return_60d,
                        return_120d: row.return_120d,
                        return_240d: row.return_240d,
                        closing_price: row.closing_price,
                    },
                }
            })
            .collect();
        Ok(Some((date, scored)))
    }
}
