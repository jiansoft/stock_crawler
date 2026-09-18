//! # 減資事件回補
//!
//! 把交易所公告的減資恢復買賣事件寫進 `corporate_action`，供 CAGR 計算調整
//! 因減資造成的價格跳動。
//!
//! ## 兩個市場的取得方式不同
//!
//! | 市場 | 來源 | 可回溯性 |
//! |------|------|----------|
//! | 上市 | TWSE `TWTAUU` | **支援日期區間**，單一請求即可取回十餘年 |
//! | 上櫃 | TPEx `bulletin/revivt` | 只回**當週**滾動公告，無法查歷史 |
//!
//! 因此上市用 [`backfill_listed_range`] 一次全量回補；上櫃只能靠
//! [`execute`] 每日排程累積，歷史缺口需另循他法（見
//! [`crate::app::backfill::capital_reduction_history`]）。
//!
//! ## 安全閥
//!
//! 減資是低頻事件（上市全市場一年僅數十筆）。若單輪解析出的筆數異常龐大，
//! 幾乎可以肯定是來源改版或解析錯誤，此時寧可整批放棄也不要污染
//! `corporate_action` —— 錯誤的比例會讓該股的報酬率整段算錯。

use anyhow::{Context, Result};
use chrono::{Datelike, Local, NaiveDate};
use scopeguard::defer;

use crate::{
    domain::performance::{CorporateAction, CorporateActionRepository},
    infra::{
        crawler::tpex, crawler::twse,
        database::repository::corporate_action::PgCorporateActionRepository,
    },
};

/// 全量回補的起始日。
///
/// 直接取端點允許的最早日（民國 100 年 1 月 1 日），比
/// [`crate::app::calculation::cagr`] 最長的十年期間更早，十年期的期初日
/// 附近也有事件可用。
///
/// **不可再往前調**：早於這一天的查詢會被 TWSE 拒絕，
/// 見 [`twse::capital_reduction::EARLIEST_QUERYABLE_DATE`]。
const FULL_BACKFILL_START: (i32, u32, u32) = twse::capital_reduction::EARLIEST_QUERYABLE_DATE;

/// 單輪允許寫入的最大筆數（安全閥）。
///
/// 上市減資一年約 15~37 筆，十餘年全量約 300 筆。單輪超過此值即視為
/// 來源異常，整批放棄。
const MAX_ACTIONS_PER_RUN: usize = 1_000;

/// 每日排程入口：抓取兩個市場當前公告中的減資事件。
///
/// - 上市：查詢「近一年」區間。區間查詢成本與查當日相同，但能順帶補回
///   服務停機期間漏掉的公告。
/// - 上櫃：`revivt` 只給當週，能抓到什麼就寫什麼。
///
/// 任一市場失敗不影響另一市場：兩者各自獨立記錄錯誤後繼續。
pub async fn execute() -> Result<()> {
    tracing::info!("減資事件回補開始");
    defer! {
        tracing::info!("減資事件回補結束");
    }

    let today = Local::now().date_naive();
    // 往回一年、往後三個月：往後是因為交易所會預先公告尚未到期的恢復買賣日，
    // 那些列此刻價格還是 `-`（會被 crawler 略過），但只要日期一到就抓得到。
    let start = today
        .with_year(today.year() - 1)
        .unwrap_or(today)
        .max(full_backfill_start());
    let end = today
        .checked_add_months(chrono::Months::new(3))
        .unwrap_or(today);

    let mut saved = 0_u64;

    match backfill_listed_range(start, end).await {
        Ok(count) => saved += count,
        Err(why) => tracing::error!("上市減資回補失敗: error={:#}", why),
    }

    match backfill_otc_current().await {
        Ok(count) => saved += count,
        Err(why) => tracing::error!("上櫃減資回補失敗: error={:#}", why),
    }

    tracing::info!("減資事件回補共寫入 {saved} 筆");

    Ok(())
}

/// 全量回補上市減資：自 [`FULL_BACKFILL_START`] 至今日。
///
/// TWSE 端點支援日期區間，因此這是單一請求就能完成的操作，
/// 供手動回補入口一次補齊歷史。
pub async fn backfill_listed_full() -> Result<u64> {
    let end = Local::now()
        .date_naive()
        .checked_add_months(chrono::Months::new(3))
        .unwrap_or_else(|| Local::now().date_naive());

    backfill_listed_range(full_backfill_start(), end).await
}

/// 回補指定區間內的上市減資事件，回傳實際寫入筆數。
pub async fn backfill_listed_range(start: NaiveDate, end: NaiveDate) -> Result<u64> {
    let actions = twse::capital_reduction::visit(start, end)
        .await
        .context("Failed to fetch listed capital reductions")?;

    tracing::info!(
        "上市減資: {start} ~ {end} 取得 {} 筆可用事件",
        actions.len()
    );

    save_all(actions, "上市").await
}

/// 回補上櫃當前公告中的減資事件，回傳實際寫入筆數。
///
/// TPEx 的公告只涵蓋當週，因此這個函式必須每日執行才不會漏。
pub async fn backfill_otc_current() -> Result<u64> {
    let actions = tpex::capital_reduction::visit()
        .await
        .context("Failed to fetch OTC capital reductions")?;

    tracing::info!("上櫃減資: 當期公告取得 {} 筆可用事件", actions.len());

    save_all(actions, "上櫃").await
}

/// 逐筆寫入 `corporate_action`，回傳成功筆數。
///
/// 單筆失敗只記錄不中斷：一檔的比例寫不進去，不該讓其餘數百筆一起放棄。
async fn save_all(actions: Vec<CorporateAction>, market: &str) -> Result<u64> {
    if actions.len() > MAX_ACTIONS_PER_RUN {
        anyhow::bail!(
            "{market}減資單輪解析出 {} 筆，超過安全閥 {MAX_ACTIONS_PER_RUN}，整批放棄以免污染 corporate_action",
            actions.len()
        );
    }

    let repository = PgCorporateActionRepository::new();
    let mut saved = 0_u64;

    for action in actions {
        match repository.save(&action).await {
            Ok(rows) => {
                saved += rows;
                tracing::debug!(
                    "{market}減資已登錄: {} {} 比率 {} {}",
                    action.stock_symbol,
                    action.effective_date,
                    action.share_ratio.round_dp(6),
                    action.note
                );
            }
            Err(why) => tracing::error!(
                "{market}減資登錄失敗: symbol={} date={} error={:#}",
                action.stock_symbol,
                action.effective_date,
                why
            ),
        }
    }

    Ok(saved)
}

/// [`FULL_BACKFILL_START`] 的 [`NaiveDate`] 形式。
fn full_backfill_start() -> NaiveDate {
    let (year, month, day) = FULL_BACKFILL_START;
    NaiveDate::from_ymd_opt(year, month, day).unwrap_or_else(|| {
        // 常數是編譯期就確定的合法日期，這條分支實務上不會走到；
        // 仍給一個保守的預設值而不是 panic。這個日期同樣不可早於
        // 民國 100 年 1 月 1 日，否則會被端點拒絕。
        NaiveDate::from_ymd_opt(2011, 1, 1).unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::performance::CorporateActionType;
    use rust_decimal_macros::dec;

    fn sample(symbol: &str) -> CorporateAction {
        CorporateAction {
            stock_symbol: symbol.to_string(),
            effective_date: NaiveDate::from_ymd_opt(1990, 1, 1).expect("測試日期應合法"),
            action_type: CorporateActionType::CapitalReduction,
            share_ratio: dec!(0.7),
            note: "減資彌補虧損".to_string(),
        }
    }

    /// 起始日必須正好是端點允許的最早日。早一天就會被 TWSE 拒絕，
    /// 全量回補會一筆都寫不進去（2026-09-18 就是這樣踩到的）。
    #[test]
    fn full_backfill_start_matches_endpoint_lower_bound() {
        assert_eq!(
            full_backfill_start(),
            NaiveDate::from_ymd_opt(2011, 1, 1).expect("測試日期應合法")
        );

        let (year, month, day) = twse::capital_reduction::EARLIEST_QUERYABLE_DATE;
        assert_eq!(
            full_backfill_start(),
            NaiveDate::from_ymd_opt(year, month, day).expect("端點下限應為合法日期")
        );
    }

    /// 安全閥：超過上限時整批放棄，且不得碰資料庫。
    #[tokio::test]
    async fn save_all_rejects_oversized_batch() {
        let actions: Vec<CorporateAction> = (0..=MAX_ACTIONS_PER_RUN)
            .map(|index| sample(&format!("{index:05}")))
            .collect();

        let result = save_all(actions, "測試").await;

        assert!(result.is_err(), "超過安全閥應回傳錯誤");
        let message = format!("{:#}", result.expect_err("已斷言為錯誤"));
        assert!(
            message.contains("安全閥"),
            "錯誤訊息應說明是安全閥擋下的：{message}"
        );
    }

    /// 空批次不應被安全閥擋下，也不該產生任何寫入。
    #[tokio::test]
    async fn save_all_accepts_empty_batch() {
        let saved = save_all(Vec::new(), "測試").await.expect("空批次應成功");

        assert_eq!(saved, 0);
    }
}
