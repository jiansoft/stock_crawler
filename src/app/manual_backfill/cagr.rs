//! 手動回補：年化報酬率（CAGR）。
//!
//! - `test_backfill_cagr_for_date`：
//!   依 [`MANUAL_CAGR_DATE`] 重算指定基準日的全市場各期間年化報酬率，寫入 `stock_cagr`。
//! - `test_backfill_cagr_period`：
//!   依 [`MANUAL_CAGR_PERIOD`] 為既有的歷史基準日回填單一統計期間（新增期間後專用）。

use chrono::NaiveDate;

use crate::app::calculation::cagr;
use crate::domain::performance::CagrPeriod;
use crate::infra::cache::SHARE;

/// 手動重算各期間年化報酬率時使用的預設基準日。
///
/// 空字串表示改用資料庫中最新的交易日。
const MANUAL_CAGR_DATE: &str = "";

/// 手動回填單一統計期間時使用的期間代碼。
///
/// 新增期間後把這裡改成該期間的代碼再執行 `test_backfill_cagr_period`。
const MANUAL_CAGR_PERIOD: &str = "Y7";

/// 手動重算指定基準日的全市場各期間年化報酬率（CAGR）。
///
/// 排程本身每日 05:40（台北時間）自動執行，這個入口用於：
/// 股利資料事後回補後需要重算、或首次上線時補算某一天的結果。
///
/// 計算完全依賴資料庫既有的報價與股利，不會呼叫任何外部網站；
/// 同一 `(基準日, 股票, 期間)` 重複執行為冪等的 upsert 覆蓋。
///
/// 執行範例：
/// `cargo test app::manual_backfill::cagr::test_backfill_cagr_for_date -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_cagr_for_date() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    // 空字串代表交由 use case 自行採用資料庫中最新的交易日。
    let date = if MANUAL_CAGR_DATE.is_empty() {
        None
    } else {
        Some(
            NaiveDate::parse_from_str(MANUAL_CAGR_DATE, "%Y-%m-%d")
                .expect("manual cagr date should be valid"),
        )
    };

    println!("開始 test_backfill_cagr_for_date date={date:?}");

    let summary = cagr::execute(date)
        .await
        .expect("manual cagr backfill failed");

    println!(
        "結束 test_backfill_cagr_for_date date={:?} universe={} periods_calculated={} periods_skipped={} rows_written={} anomaly_symbols={}",
        summary.date,
        summary.universe,
        summary.periods_calculated,
        summary.periods_skipped,
        summary.rows_written,
        summary.anomaly_symbols
    );
}

/// 為既有的歷史基準日回填單一統計期間（[`MANUAL_CAGR_PERIOD`]）。
///
/// 新增期間之後專用：排程只算當日，既有基準日不會自動長出新期間的資料。
/// 此入口掃出「已有其他期間結果、但缺少該期間」的基準日逐日補算，
/// **只算指定期間**，不重算已存在的其他期間。
///
/// 從未計算過的日期不在範圍內；要初始化那些日期請改用
/// [`test_backfill_cagr_for_date`] 逐日執行。
///
/// 全程只讀資料庫既有的報價與股利，不呼叫任何外部網站；重複執行為冪等。
///
/// 執行範例：
/// `cargo test app::manual_backfill::cagr::test_backfill_cagr_period -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_cagr_period() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let period =
        CagrPeriod::from_code(MANUAL_CAGR_PERIOD).expect("manual cagr period 應為合法代碼");

    println!("開始 test_backfill_cagr_period period={}", period.code());

    let summary = cagr::backfill_period(period)
        .await
        .expect("manual cagr period backfill failed");

    println!(
        "結束 test_backfill_cagr_period period={} dates_pending={} dates_processed={} dates_skipped={} rows_written={}",
        period.code(),
        summary.dates_pending,
        summary.dates_processed,
        summary.dates_skipped,
        summary.rows_written
    );
}
