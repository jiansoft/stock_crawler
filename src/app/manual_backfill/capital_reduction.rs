//! 手動回補：減資事件。
//!
//! - `test_backfill_listed_capital_reductions`：
//!   自 2010 年起全量回補**上市**減資事件到 `corporate_action`。TWSE 端點支援
//!   日期區間，單一請求即可取回十餘年，因此這是一次性的補歷史操作。
//! - `test_backfill_otc_capital_reductions`：
//!   回補**上櫃**當期公告的減資事件。TPEx 只給當週資料，這個入口僅用於
//!   排程漏跑時的補救，補不回更早的歷史。
//! - `test_scan_otc_capital_reduction_gaps`：
//!   **只讀**。列出上櫃仍未解釋的價格跳動，收斂成逐檔待辦清單，
//!   供人工登錄或未來接上逐檔來源時當目標清單。

use chrono::NaiveDate;

use crate::app::backfill::capital_reduction;
use crate::app::backfill::capital_reduction_history;
use crate::infra::cache::SHARE;

/// 掃描上櫃減資歷史缺口的起始日。
///
/// 與 CAGR 最長的十年期間對齊並多留數年緩衝。
const MANUAL_CAPITAL_REDUCTION_SCAN_FROM: &str = "2014-01-01";

/// 全量回補上市減資事件。
///
/// TWSE 的 `TWTAUU` 端點支援 `startDate`／`endDate` 區間查詢，
/// **單一請求**就能取回 2010 年至今的全部事件（實測 2015–2026 為 308 筆），
/// 因此不需要逐日輪詢。跑完後建議重跑 `test_backfill_cagr_for_date`，
/// 讓新登錄的減資反映到年化報酬率上。
///
/// `cargo test app::manual_backfill::capital_reduction::test_backfill_listed_capital_reductions -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_listed_capital_reductions() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    println!("開始 test_backfill_listed_capital_reductions");

    let saved = capital_reduction::backfill_listed_full()
        .await
        .expect("manual listed capital reduction backfill failed");

    println!("結束 test_backfill_listed_capital_reductions rows_written={saved}");
}

/// 回補上櫃當期公告的減資事件。
///
/// TPEx 的 `revivt` 只回當週公告且不接受日期參數，所以這個入口**補不回歷史**，
/// 僅供排程漏跑當週時補救。歷史缺口請見
/// [`crate::app::backfill::capital_reduction_history`]。
///
/// `cargo test app::manual_backfill::capital_reduction::test_backfill_otc_capital_reductions -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_otc_capital_reductions() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    println!("開始 test_backfill_otc_capital_reductions");

    let saved = capital_reduction::backfill_otc_current()
        .await
        .expect("manual OTC capital reduction backfill failed");

    println!("結束 test_backfill_otc_capital_reductions rows_written={saved}");
}

/// 掃描上櫃減資的歷史缺口，列出待辦清單。
///
/// **只讀，不寫任何資料。** 上櫃沒有可回溯的減資來源（TPEx 的公告只涵蓋當週），
/// 而比例無法從報價反推——漲跌停只能把它框在 ±10% 區間，減資比例又不是整數倍，
/// 框不出唯一解。寧可留白也不要寫入猜測值。
///
/// 建議在 `test_backfill_listed_capital_reductions` 之後執行，此時清單中
/// 才不會混入已由 TWSE 全量覆蓋的上市部分。
///
/// `cargo test app::manual_backfill::capital_reduction::test_scan_otc_capital_reduction_gaps -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_scan_otc_capital_reduction_gaps() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let to = chrono::Local::now().date_naive();
    let from = NaiveDate::parse_from_str(MANUAL_CAPITAL_REDUCTION_SCAN_FROM, "%Y-%m-%d")
        .expect("manual capital reduction scan from should be valid");

    println!("開始 test_scan_otc_capital_reduction_gaps from={from} to={to}");

    let report = capital_reduction_history::scan(from, to)
        .await
        .expect("manual capital reduction gap scan failed");

    println!("{}", capital_reduction_history::format_report(&report));
    println!(
        "結束 test_scan_otc_capital_reduction_gaps symbols={} events={}",
        report.symbol_count(),
        report.event_count()
    );
}
