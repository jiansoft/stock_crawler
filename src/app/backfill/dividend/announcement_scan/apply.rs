//! # 異動計畫的套用
//!
//! 把 [`super::plan`] 彙整出來的計畫逐筆寫進資料庫，並統計本次掃描的成果；
//! 兩階段都判不出期別的事件在這裡彙總成 log，不會寫入任何資料。
//!
//! 這是整條流程唯一會改動資料庫的地方，採 best-effort：單筆失敗只記錄錯誤並繼續，
//! 不讓某一檔的問題拖垮整次掃描。

use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::{app::calculation::dividend_record, domain::dividend::repository::DividendRepository};

use super::{
    ScanOutcome,
    plan::ScanPlan,
    quarter::{UnresolvedEvent, UnresolvedReason},
};

/// 逐筆列出「無法判定期別」事件的上限。
const UNRESOLVED_SAMPLE_LIMIT: usize = 10;

/// 逐筆執行異動計畫。
///
/// 單筆失敗只記錄錯誤並繼續處理下一筆：一次掃描涵蓋全市場，
/// 不該因為某一檔的問題就讓其餘更新全部落空。
///
/// 新增分期事件後必須跟著重算年度合計與持股股利明細，
/// 與既有的 [`crate::app::backfill::dividend::missing_or_multiple`] 流程一致；少了這兩步，
/// 年度合計會停在補進來之前的數字。
pub(super) async fn apply_plan(
    repository: &dyn DividendRepository,
    plan: ScanPlan,
) -> Result<ScanOutcome> {
    let mut outcome = ScanOutcome {
        unresolved: plan.unresolved.len(),
        ..Default::default()
    };

    for update in plan.updates.values() {
        match repository.update_dividend_date(update).await {
            Ok(_) => outcome.updated += 1,
            Err(why) => tracing::error!(
                "更新 {} (serial {}) 的除權息日期失敗: {:?}",
                update.security_code,
                update.serial,
                why
            ),
        }
    }

    // 新增成功後要重算的目標；年度合計以 (代號, 發放年度) 為單位，持股明細以代號為單位。
    let mut annual_total_targets: HashSet<(String, i32)> = HashSet::new();
    let mut record_targets: HashSet<String> = HashSet::new();

    for dividend in plan.inserts.values() {
        match repository.save(dividend).await {
            Ok(_) => {
                outcome.inserted += 1;
                record_targets.insert(dividend.security_code.clone());
                if !dividend.quarter.is_empty() {
                    // 分期事件會改變年度合計列，記下來稍後統一重算。
                    annual_total_targets.insert((dividend.security_code.clone(), dividend.year));
                }
                tracing::info!(
                    "新增漏抓的股利事件 {} {}{} 除權息日 {}",
                    dividend.security_code,
                    dividend.year_of_dividend,
                    if dividend.quarter.is_empty() {
                        "年度".to_string()
                    } else {
                        dividend.quarter.clone()
                    },
                    dividend.ex_dividend_date_cash
                );
            }
            Err(why) => {
                tracing::error!("新增 {} 的股利資料失敗: {:?}", dividend.security_code, why)
            }
        }
    }

    for (security_code, year) in &annual_total_targets {
        if let Err(why) = repository
            .upsert_annual_total_dividend(security_code, *year)
            .await
        {
            tracing::error!(
                "重算 {} {} 年度合計股利失敗: {:?}",
                security_code,
                year,
                why
            );
        }
    }

    for security_code in &record_targets {
        if let Err(why) =
            dividend_record::backfill_received_dividend_records_for_stock(security_code).await
        {
            tracing::error!("重算 {} 的持股已領股利失敗: {:?}", security_code, why);
        }
    }

    log_unresolved(&plan.unresolved);

    Ok(outcome)
}

/// 依原因彙總無法判定期別的事件。
///
/// 兩階段都判不出來的事件才會走到這裡。逐筆最多列
/// [`UNRESOLVED_SAMPLE_LIMIT`] 筆，其餘只記總數，避免一次塞爆 log。
fn log_unresolved(events: &[UnresolvedEvent]) {
    if events.is_empty() {
        return;
    }

    let mut counts: HashMap<UnresolvedReason, usize> = HashMap::new();
    for event in events {
        *counts.entry(event.reason).or_default() += 1;
    }
    let summary: Vec<String> = counts
        .iter()
        .map(|(reason, count)| format!("{}={}", reason.as_str(), count))
        .collect();

    tracing::info!(
        total = events.len(),
        detail = summary.join("、"),
        "除權息事件無法判定期別，未寫入資料庫"
    );

    for event in events.iter().take(UNRESOLVED_SAMPLE_LIMIT) {
        tracing::warn!(
            "{} {} 無法判定期別：{}",
            event.announcement.stock_symbol,
            event.announcement.ex_date,
            event.reason.as_str()
        );
    }
}
