use anyhow::Result;
use chrono::{Datelike, Local};
use scopeguard::defer;

use crate::logging;

mod missing_or_multiple;
pub mod payout_ratio;
mod unannounced_ex_dividend_date;

use missing_or_multiple::backfill_missing_or_multiple_dividends;
use unannounced_ex_dividend_date::backfill_unannounced_dividend_dates;

/// 執行年度股利回補（backfill）主流程。
///
/// 這個入口會以「今年」為處理範圍，並行執行兩條子流程：
/// 1. `backfill_missing_or_multiple_dividends`：
///    補抓「當年度尚無股利資料」或「當年度已有多筆配息紀錄」的股票。
/// 2. `backfill_unannounced_dividend_dates`：
///    補抓「除息日/發放日尚未公告」的既有股利資料。
///
/// 子流程以 `tokio::join!` 併發執行，互不阻塞；任一子流程失敗不會中止另一條。
/// 每條子流程的錯誤都會寫入 log，流程最後統一返回。
///
/// # 設計說明
///
/// - 以 `Local::now().year()` 作為年度基準。
/// - 使用 `scopeguard::defer!` 保證「結束」log 在函式離開時一定會寫出。
/// - 採用 best-effort 策略：偏重資料補齊與可觀測性，而不是 fail-fast。
///
/// # Returns
///
/// - `Ok(())`：主流程執行完成（即使某些子流程失敗，仍以 log 記錄後返回）。
///
/// # Errors
///
/// 目前實作不會把子流程錯誤向上拋出；子流程的 `Err` 會在本函式內被記錄。
/// `Result<()>` 型別保留為介面一致性與未來擴充（例如改為聚合錯誤回傳）。
pub async fn execute() -> Result<()> {
    // 進入主流程先寫開始 log，方便排程任務追蹤一次執行的起點。
    logging::info_file_async("更新台股股利發放數據開始");
    defer! {
       // 無論中途是否發生錯誤、提早返回或 panic unwind，都嘗試補上結束 log。
       // 這樣可以確保「開始/結束」成對，方便觀察是否有卡住或異常中斷。
       logging::info_file_async("更新台股股利發放數據結束");
    }

    // 以本地時間的「今年」當作回補目標年度。
    let now = Local::now();
    let year = now.year();

    // 兩條流程都依賴同一年度參數，但互相獨立，適合併行縮短整體耗時。
    let backfill_missing_or_multiple_dividends_task = backfill_missing_or_multiple_dividends(year);
    let backfill_unannounced_dividend_dates_task = backfill_unannounced_dividend_dates(year);

    // join! 會同時等待兩條流程完成；這裡選擇 best-effort，不因單一路徑失敗而取消另一條。
    let (res_backfill_missing_or_multiple_dividends, res_backfill_unannounced_dividend_dates) =
        tokio::join!(
            backfill_missing_or_multiple_dividends_task,
            backfill_unannounced_dividend_dates_task
        );

    // 子流程結果各自記錄，避免只看到一個總錯誤而失去定位資訊。
    match res_backfill_missing_or_multiple_dividends {
        Ok(_) => {
            logging::info_file_async(
                "backfill_missing_or_multiple_dividends executed successfully.".to_string(),
            );
        }
        Err(why) => {
            logging::error_file_async(format!(
                "Failed to backfill_missing_or_multiple_dividends because {:?}",
                why
            ));
        }
    }

    // 第二條流程同樣採「記錄錯誤但不中斷主流程」策略，優先確保回補任務整體可完成。
    match res_backfill_unannounced_dividend_dates {
        Ok(_) => {
            logging::info_file_async(
                "backfill_unannounced_dividend_dates executed successfully.".to_string(),
            );
        }
        Err(why) => {
            logging::error_file_async(format!(
                "Failed to backfill_unannounced_dividend_dates because {:?}",
                why
            ));
        }
    }

    Ok(())
}
