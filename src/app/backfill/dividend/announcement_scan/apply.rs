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

    tracing::info!(
        total = events.len(),
        awaiting_amount = awaiting.len(),
        detail = summary.join("、"),
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
        tracing::info!(
            "金額待公告、Yahoo 尚未列出，等公告後再補：{}",
            symbols.join("、")
        );
    }

    for event in attention.iter().take(UNRESOLVED_SAMPLE_LIMIT) {
        tracing::warn!(
            "{} {} 無法判定期別：{}",
            event.announcement.stock_symbol,
            event.announcement.ex_date,
            event.reason.as_str()
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
mod tests {
    use rust_decimal_macros::dec;

    use super::super::fixtures::announcement;
    use super::*;

    fn event(
        cash: Option<rust_decimal::Decimal>,
        ex_date: (i32, u32, u32),
        reason: UnresolvedReason,
    ) -> UnresolvedEvent {
        UnresolvedEvent {
            announcement: announcement("00904", ex_date, true, false, cash, None),
            reason,
        }
    }

    /// 2026-10-09 的 00904：10-16 除息、金額待公告、Yahoo 未列出 → 只是在等公告。
    #[test]
    fn test_is_awaiting_amount_far_from_ex_date() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        let item = event(
            None,
            (2026, 10, 16),
            UnresolvedReason::NoMatchingYahooDividend,
        );

        assert!(is_awaiting_amount(&item, today));
    }

    /// 剩 3 天內還判不出來就要告警，免得錯過除息提醒。
    #[test]
    fn test_is_awaiting_amount_warns_near_ex_date() {
        let item = event(
            None,
            (2026, 10, 16),
            UnresolvedReason::NoMatchingYahooDividend,
        );

        let near = NaiveDate::from_ymd_opt(2026, 10, 13).unwrap();
        assert!(!is_awaiting_amount(&item, near));
        let day_before = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        assert!(is_awaiting_amount(&item, day_before));
    }

    /// 金額已公布卻對不上、或 Yahoo 抓取失敗，都不是等待狀態。
    #[test]
    fn test_is_awaiting_amount_requires_pending_amount_and_yahoo_miss() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();

        let announced = event(
            Some(dec!(0.5)),
            (2026, 10, 16),
            UnresolvedReason::NoMatchingYahooDividend,
        );
        assert!(!is_awaiting_amount(&announced, today));

        let failed = event(None, (2026, 10, 16), UnresolvedReason::YahooLookupFailed);
        assert!(!is_awaiting_amount(&failed, today));
    }

    /// 寫入計畫：更新日期、新增分期並重算年度合計；無法判定期別的事件只計數不寫入。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
    )]
    async fn apply_plan_updates_inserts_and_rebuilds_annual_total() {
        use std::collections::BTreeMap;

        use super::super::fixtures::existing_row;
        use crate::infra::database::repository::dividend::PgDividendRepository;

        const SYMBOL: &str = "79979AS";
        dotenvy::dotenv().ok();
        let pool = crate::infra::database::get_connection();
        if sqlx::query("SELECT 1").execute(pool).await.is_err() {
            println!("跳過 apply_plan_updates_inserts_and_rebuilds_annual_total：無資料庫連接");
            return;
        }
        let cleanup = || async {
            sqlx::query("DELETE FROM dividend WHERE security_code = $1")
                .bind(SYMBOL)
                .execute(pool)
                .await
                .expect("清除測試股利");
        };
        cleanup().await;

        let repo = PgDividendRepository::new();
        let mut first = existing_row(0, SYMBOL, 1991, "Q1", "-", "-");
        first.cash_dividend = dec!(1);
        first.sum = dec!(1);
        repo.save(&first).await.expect("種入第一季");
        let mut update = repo
            .fetch_by_years(&[1991])
            .await
            .expect("讀取股利")
            .into_iter()
            .find(|row| row.security_code == SYMBOL)
            .expect("讀回第一季");
        update.ex_dividend_date_cash = "1991-04-10".to_string();
        let mut second = existing_row(0, SYMBOL, 1991, "Q2", "1991-07-10", "-");
        second.cash_dividend = dec!(2);
        second.sum = dec!(2);

        let plan = ScanPlan {
            updates: BTreeMap::from([(update.serial, update)]),
            inserts: BTreeMap::from([((SYMBOL.to_string(), 1991, 1990, "Q2".to_string()), second)]),
            unresolved: vec![
                event(
                    None,
                    (2099, 1, 1),
                    UnresolvedReason::NoMatchingYahooDividend,
                ),
                event(None, (2099, 1, 1), UnresolvedReason::YahooLookupFailed),
            ],
        };
        let outcome = apply_plan(&repo, plan).await.expect("套用計畫");
        assert_eq!(
            (outcome.updated, outcome.inserted, outcome.unresolved),
            (1, 1, 2)
        );

        let rows: Vec<_> = repo
            .fetch_by_years(&[1991])
            .await
            .expect("重新讀取")
            .into_iter()
            .filter(|row| row.security_code == SYMBOL)
            .collect();
        let quarter = |q: &str| rows.iter().find(|row| row.quarter == q);
        assert_eq!(
            quarter("Q1").expect("第一季").ex_dividend_date_cash,
            "1991-04-10"
        );
        assert_eq!(quarter("Q2").expect("第二季").cash_dividend, dec!(2));
        let total = quarter("").or_else(|| quarter("A")).expect("年度合計列");
        assert_eq!(total.cash_dividend, dec!(3));

        cleanup().await;
    }
}
