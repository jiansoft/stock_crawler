//! 訊號績效的大盤基準：同市場全部股票各天期的平均報酬與上漲比例，可依年度拆分。

use chrono::NaiveDate;

use super::HORIZONS;
use super::rows::BenchmarkRow;
use super::sql::{BENCHMARK_BY_YEAR_SQL, BENCHMARK_SQL};
use crate::infra::database;
use crate::interfaces::web::data_api::dto::{BenchmarkStat, BenchmarkYear};
use crate::interfaces::web::data_api::handlers::{round2, round4};

/// 期間內的大盤基準；`by_year` 時一併取各年度的基準。
pub(in crate::interfaces::web::data_api::handlers) struct Benchmark {
    pub(super) total: BenchmarkRow,
    pub(super) years: Option<Vec<BenchmarkRow>>,
}

/// 查詢大盤基準；`by_year` 時用 [`BENCHMARK_BY_YEAR_SQL`] 一次取整段與各年度。
pub(in crate::interfaces::web::data_api::handlers) async fn fetch_benchmark(
    from: NaiveDate,
    to: NaiveDate,
    market_id: i32,
    by_year: bool,
) -> Result<Benchmark, sqlx::Error> {
    let pool = database::get_connection();
    // 不快取成具名 prepared statement：重複執行後 PostgreSQL 改用 generic plan 時，`$3 = 0 OR` 的
    // 市場條件剪不掉（2026-10-28 正式庫實測三年依年度 custom 2.1 秒、generic 5.7 秒）。
    if !by_year {
        let total = sqlx::query_as(BENCHMARK_SQL)
            .bind(from)
            .bind(to)
            .bind(market_id)
            .persistent(false)
            .fetch_one(pool)
            .await?;
        return Ok(Benchmark { total, years: None });
    }
    let rows: Vec<BenchmarkRow> = sqlx::query_as(BENCHMARK_BY_YEAR_SQL)
        .bind(from)
        .bind(to)
        .bind(market_id)
        .persistent(false)
        .fetch_all(pool)
        .await?;
    Ok(split_benchmark(rows))
}

/// 把 [`BENCHMARK_BY_YEAR_SQL`] 的結果分成整段期間與各年度（由舊到新）。
pub(super) fn split_benchmark(rows: Vec<BenchmarkRow>) -> Benchmark {
    let (totals, mut years): (Vec<_>, Vec<_>) =
        rows.into_iter().partition(|row| row.year.is_none());
    years.sort_by_key(|row| row.year);
    Benchmark {
        total: totals.into_iter().next().unwrap_or_default(),
        years: Some(years),
    }
}

/// 整段期間與各年度的大盤基準統計；沒有依年度拆分時第二項為 `None`。
pub(in crate::interfaces::web::data_api::handlers) fn benchmark_output(
    benchmark: &Benchmark,
) -> (Vec<BenchmarkStat>, Option<Vec<BenchmarkYear>>) {
    let years = benchmark.years.as_ref().map(|years| {
        years
            .iter()
            .filter_map(|row| {
                Some(BenchmarkYear {
                    year: row.year?,
                    benchmark: benchmark_stats(row),
                })
            })
            .collect()
    });
    (benchmark_stats(&benchmark.total), years)
}

/// 大盤基準的各天期統計。
pub(super) fn benchmark_stats(benchmark: &BenchmarkRow) -> Vec<BenchmarkStat> {
    [
        (benchmark.n5, benchmark.avg5, benchmark.win5),
        (benchmark.n20, benchmark.avg20, benchmark.win20),
        (benchmark.n60, benchmark.avg60, benchmark.win60),
        (benchmark.n120, benchmark.avg120, benchmark.win120),
        (benchmark.n240, benchmark.avg240, benchmark.win240),
    ]
    .into_iter()
    .zip(HORIZONS)
    .map(|((samples, avg, win), days)| BenchmarkStat {
        days,
        samples,
        avg_return: avg.map(round2),
        win_rate: win.map(round4),
    })
    .collect()
}
