//! 訊號績效的分組平行查詢：依「來源×種類」拆成多個查詢同時執行，再把各組的列合併。
//!
//! 統計依（來源, 種類）與（來源, 種類, 年度）分組，所以每一組單獨查詢的結果與整段一次查完全相同。
//! 一次查時全部訊號的中位數（ordered-set aggregate）不能平行，只能單核心排序：2026-10-28 正式庫
//! 依年度三年 12.3 秒（GroupAggregate 佔 8.6 秒）；拆成 24 組後最慢的一組（kd／oversold）1.8 秒，
//! 加總 20 秒（每組都要重算大盤基準 CTE），同時跑 [`PARALLELISM`] 組。

use chrono::NaiveDate;
use futures::{StreamExt, TryStreamExt};

use super::rows::PerformanceRow;
use super::sql::SIGNAL_COMBOS_SQL;
use crate::infra::database;

/// 同時執行的組數：訊號績效走 Data API 重型連線池（上限 8），另留連線給同時查詢的大盤基準。
const PARALLELISM: usize = 6;

/// 訊號績效 SQL 的篩選參數（`$1`～`$8`，有籌碼條件時再加 `$9`／`$10`）。
#[derive(Debug, Clone, Copy)]
pub(super) struct PerformanceFilter<'a> {
    pub(super) from: NaiveDate,
    pub(super) to: NaiveDate,
    pub(super) market_id: i32,
    pub(super) indicator: Option<&'a str>,
    pub(super) kind: Option<&'a str>,
    pub(super) industry: Option<&'a str>,
    pub(super) min_rs_score: Option<i16>,
    pub(super) max_rs_score: Option<i16>,
    /// 外資／投信連續買超下限；`None` 時用沒有籌碼條件的 SQL（不綁 `$9`／`$10`）。
    pub(super) streaks: Option<(Option<i64>, Option<i64>)>,
}

/// 依期間內出現過的來源×種類分組平行執行 `sql`，回傳合併後的列（順序不固定，由呼叫端排序）。
pub(super) async fn fetch_performance_rows(
    sql: &'static str,
    filter: PerformanceFilter<'_>,
) -> Result<Vec<PerformanceRow>, sqlx::Error> {
    let combos: Vec<(String, String)> = sqlx::query_as(SIGNAL_COMBOS_SQL)
        .bind(filter.from)
        .bind(filter.to)
        .bind(filter.indicator)
        .bind(filter.kind)
        .fetch_all(database::get_connection())
        .await?;
    let parts: Vec<Vec<PerformanceRow>> = futures::stream::iter(combos)
        .map(|(indicator, kind)| fetch_combo(sql, filter, indicator, kind))
        .buffer_unordered(PARALLELISM)
        .try_collect()
        .await?;
    Ok(parts.into_iter().flatten().collect())
}

/// 只查一組來源×種類（取代請求原本的來源／種類條件，其餘篩選照舊）。
async fn fetch_combo(
    sql: &'static str,
    filter: PerformanceFilter<'_>,
    indicator: String,
    kind: String,
) -> Result<Vec<PerformanceRow>, sqlx::Error> {
    let mut query = sqlx::query_as(sql)
        .bind(filter.from)
        .bind(filter.to)
        .bind(filter.market_id)
        .bind(indicator)
        .bind(kind)
        .bind(filter.industry)
        .bind(filter.min_rs_score)
        .bind(filter.max_rs_score);
    if let Some((foreign, trust)) = filter.streaks {
        query = query.bind(foreign).bind(trust);
    }
    // 不快取成具名 prepared statement：同一連線執行約 5 次後 PostgreSQL 會改用 generic plan，可選條件
    // 剪不掉（2026-10-28 正式庫實測近一年全市場 custom 3.4 秒、generic 56 秒）。Data API 連線池也已
    // 強制 custom plan，這裡再明確標示一次。
    query
        .persistent(false)
        .fetch_all(database::get_connection())
        .await
}
