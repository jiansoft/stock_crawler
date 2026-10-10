use anyhow::Result;
use chrono::{Datelike, Local};
use scopeguard::defer;

/// 每日掃描交易所除權息公告，補齊漏抓的股利事件。
mod announcement_scan;
/// 修復帶有配息日期的年度合計列。
mod annual_total_repair;
/// 以交易所除權除息計算結果核對股利資料。
pub(crate) mod ex_right_reconcile;
/// 手動回補 Yahoo 歷年配息明細。
mod historical;
/// 定期掃描近期股利，補齊缺漏或新增的配息。
mod missing_or_multiple;
/// 更新歷史配息率。
pub mod payout_ratio;
mod unannounced_ex_dividend_date;
/// Yahoo 股利政策頁的共用重試抓取。
mod yahoo_fetch;

use missing_or_multiple::backfill_missing_or_multiple_dividends;
use unannounced_ex_dividend_date::backfill_unannounced_dividend_dates;

/// 年度合計列修復的手動入口。
pub(crate) use annual_total_repair::repair_stale_annual_total_dividends;

/// 單檔歷年股利手動回補入口。
pub(crate) use historical::{
    backfill_historical_dividends_for_multiple_dividend_stocks,
    backfill_historical_dividends_for_stock,
};

/// 執行年度股利回補（backfill）主流程。
///
/// 這個入口會以「今年」為處理範圍，並行執行兩條子流程：
/// 1. `backfill_missing_or_multiple_dividends`：
///    定期檢查上市櫃股票的近期股利，包含原本年配但後續新增半年配的股票。
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
    tracing::info!("更新台股股利發放數據開始");
    defer! {
       // 無論中途是否發生錯誤、提早返回或 panic unwind，都嘗試補上結束 log。
       // 這樣可以確保「開始/結束」成對，方便觀察是否有卡住或異常中斷。
       tracing::info!("更新台股股利發放數據結束");
    }

    // 以本地時間的「今年」當作回補目標年度。
    let now = Local::now();
    let year = now.year();

    // 兩條流程都依賴同一年度參數，但互相獨立，適合併行縮短整體耗時。
    let backfill_missing_or_multiple_dividends_task = backfill_missing_or_multiple_dividends(year);
    let backfill_unannounced_dividend_dates_task = backfill_unannounced_dividend_dates(year);

    // join! 會同時等待兩條流程完成；這裡選擇 best-effort，不因單一路徑失敗而取消另一條。
    let (res_backfill_missing_or_multiple_dividends, res_backfill_unannounced_dividend_dates) = tokio::join!(
        backfill_missing_or_multiple_dividends_task,
        backfill_unannounced_dividend_dates_task
    );

    // 子流程結果各自記錄，避免只看到一個總錯誤而失去定位資訊。
    match res_backfill_missing_or_multiple_dividends {
        Ok(_) => {
            tracing::info!(
                "{}",
                "backfill_missing_or_multiple_dividends executed successfully.".to_string(),
            );
        }
        Err(why) => {
            tracing::error!(
                "Failed to backfill_missing_or_multiple_dividends because {:?}",
                why
            );
        }
    }

    // 第二條流程同樣採「記錄錯誤但不中斷主流程」策略，優先確保回補任務整體可完成。
    match res_backfill_unannounced_dividend_dates {
        Ok(_) => {
            tracing::info!(
                "{}",
                "backfill_unannounced_dividend_dates executed successfully.".to_string(),
            );
        }
        Err(why) => {
            tracing::error!(
                "Failed to backfill_unannounced_dividend_dates because {:?}",
                why
            );
        }
    }

    Ok(())
}

/// 以交易所除權除息計算結果核對近 45 天的股利資料。
///
/// 見 [`ex_right_reconcile`]；交易所來源抓取失敗時回傳錯誤、不寫入任何資料。
pub async fn reconcile_ex_right_results() -> Result<()> {
    tracing::info!("核對交易所除權息結果開始");
    defer! {
       tracing::info!("核對交易所除權息結果結束");
    }

    let today = Local::now().date_naive();
    let start = today - chrono::Duration::days(RECONCILE_LOOKBACK_DAYS);
    let summary = ex_right_reconcile::execute(start, today, true).await?;
    tracing::info!(
        official = summary.official,
        matched = summary.matched,
        updated = summary.updated,
        missing = summary.missing,
        ambiguous = summary.ambiguous,
        filled = summary.filled,
        stock_filled = summary.stock_filled,
        exchange_filled = summary.exchange_filled,
        unresolved = summary.unresolved,
        "核對交易所除權息結果完成"
    );
    Ok(())
}

/// 每日核對回看的天數：延後除息、日期未公布的列通常在除權息後幾週內才會被發現。
const RECONCILE_LOOKBACK_DAYS: i64 = 45;

/// 掃描交易所的除權除息公告，補齊漏抓的股利事件與日期。
///
/// 與 [`execute`] 的差別在於資料流向：`execute` 是從資料庫既有資料出發去補欄位，
/// 這裡則是從交易所的全市場公告出發，因此能發現「資料庫根本沒有」的事件。
/// 兩者互補，各自獨立排程。
///
/// # Errors
///
/// 只有在上市與上櫃的除權息預告表都取不到時才回傳錯誤；
/// 單筆資料的更新失敗會記錄於 log 並繼續處理其餘資料。
pub async fn scan_announcements() -> Result<()> {
    tracing::info!("掃描除權息公告開始");
    defer! {
       tracing::info!("掃描除權息公告結束");
    }

    let outcome = announcement_scan::scan_ex_dividend_announcements().await?;
    tracing::info!(
        updated = outcome.updated,
        inserted = outcome.inserted,
        unresolved = outcome.unresolved,
        "掃描除權息公告完成"
    );

    Ok(())
}

/// 判斷代號是否為 ETN（指數投資證券，代號為 `02` 開頭的六碼）。
///
/// ETN 的配息不收錄在 `dividend` 資料表（表中沒有任何 ETN），Yahoo 也沒有 ETN 的
/// 股利政策頁。除權息公告掃描與交易所核對都要先排除：留著只會變成「無法判定期別」
/// 或「資料庫找不到」的假警告，並多一次注定失敗的 Yahoo 請求（020035 在核對流程曾每天記 2 筆）。
fn is_exchange_traded_note(symbol: &str) -> bool {
    symbol.len() == 6 && symbol.starts_with("02") && symbol.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_exchange_traded_note() {
        assert!(is_exchange_traded_note("020035"));
        assert!(is_exchange_traded_note("020001"));
        assert!(!is_exchange_traded_note("0050"));
        assert!(!is_exchange_traded_note("00400A"));
        assert!(!is_exchange_traded_note("2890"));
        assert!(!is_exchange_traded_note("02003B"));
    }
}
