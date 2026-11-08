//! # 異動計畫的套用
//!
//! 把 [`super::plan`] 彙整出來的計畫逐筆寫進資料庫，並統計本次掃描的成果；
//! 兩階段都判不出期別的事件在這裡彙總成 log，不會寫入任何資料。
//!
//! 這是整條流程唯一會改動資料庫的地方，採 best-effort：單筆失敗只記錄錯誤並繼續，
//! 不讓某一檔的問題拖垮整次掃描。

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use chrono::{Local, NaiveDate};

use crate::{app::calculation::dividend_record, domain::dividend::repository::DividendRepository};

use super::{
    ScanOutcome,
    plan::ScanPlan,
    quarter::{UnresolvedEvent, UnresolvedReason},
};

/// 逐筆列出「無法判定期別」事件的上限。
const UNRESOLVED_SAMPLE_LIMIT: usize = 10;

/// 金額待公告的事件，離除權息日剩幾天（含）以內仍判不出期別才發 warn。
///
/// ETF 預告表常先登「待公告實際收益分配金額」，Yahoo 要等發行商公布金額才會列出
/// 這次配息，因此在那之前每天都會判不出期別（2026-10 的 00904 連續 warn 一週多）。
/// 金額公布後下一次排程就會補上；只有快到除權息日還沒補到才需要人處理。
/// 3 天涵蓋週末：週五的除權息日在週二就會開始告警。
const AWAITING_AMOUNT_WARN_DAYS: i64 = 3;

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
                let quarter = if dividend.quarter.is_empty() {
                    "年度"
                } else {
                    dividend.quarter.as_str()
                };
                tracing::info!(
                    "新增漏抓的股利事件 {} {}{} 除權息日 {}",
                    dividend.security_code,
                    dividend.year_of_dividend,
                    quarter,
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

    log_unresolved(&plan.unresolved, Local::now().date_naive());

    Ok(outcome)
}

/// 依原因彙總無法判定期別的事件。
///
/// 兩階段都判不出來的事件才會走到這裡。需要人處理的逐筆以 warn 最多列
/// [`UNRESOLVED_SAMPLE_LIMIT`] 筆；金額尚未公告、離除權息日還遠的事件屬於正常的
/// 等待狀態（見 [`is_awaiting_amount`]），只記 info。
fn log_unresolved(events: &[UnresolvedEvent], today: NaiveDate) {
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

    let (awaiting, attention): (Vec<&UnresolvedEvent>, Vec<&UnresolvedEvent>) = events
        .iter()
        .partition(|event| is_awaiting_amount(event, today));

    let event_total = events.len();
    let awaiting_total = awaiting.len();
    let detail = summary.join("、");
    tracing::info!(
        total = event_total,
        awaiting_amount = awaiting_total,
        detail,
        "除權息事件無法判定期別，未寫入資料庫"
    );

    if !awaiting.is_empty() {
        let symbols: Vec<String> = awaiting
            .iter()
            .map(|event| {
                format!(
                    "{} {}",
                    event.announcement.stock_symbol, event.announcement.ex_date
                )
            })
            .collect();
        let pending = symbols.join("、");
        tracing::info!("金額待公告、Yahoo 尚未列出，等公告後再補：{}", pending);
    }

    for event in attention.iter().take(UNRESOLVED_SAMPLE_LIMIT) {
        let reason = event.reason.as_str();
        tracing::warn!(
            "{} {} 無法判定期別：{}",
            event.announcement.stock_symbol,
            event.announcement.ex_date,
            reason
        );
    }
}

/// 判斷事件是否只是在等發行商公告金額。
///
/// 條件是 Yahoo 已查過但沒有這次配息、預告表的金額也還沒公布，且離除權息日超過
/// [`AWAITING_AMOUNT_WARN_DAYS`] 天。Yahoo 抓取失敗等其他原因不算，照常告警。
fn is_awaiting_amount(event: &UnresolvedEvent, today: NaiveDate) -> bool {
    let announcement = &event.announcement;
    let amount_pending = (announcement.is_cash && announcement.cash_dividend.is_none())
        || (announcement.is_stock && announcement.stock_dividend_ratio.is_none());

    event.reason == UnresolvedReason::NoMatchingYahooDividend
        && amount_pending
        && (announcement.ex_date - today).num_days() > AWAITING_AMOUNT_WARN_DAYS
}

#[cfg(test)]
mod tests;
