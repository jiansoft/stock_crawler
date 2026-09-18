//! # 第二階段：逐檔查詢 Yahoo 補救期別
//!
//! [`super::quarter`] 的第一階段只涵蓋上市公司（MOPS 沒有上櫃與 ETF 的開放資料），
//! 配不出期別的事件都會落到這裡，改用 Yahoo 的股利政策頁逐檔補救。
//! 兩階段的分工與理由見 [`super`] 的「期別怎麼來：兩階段判定」。
//!
//! 補救成功的事件會回頭走一次 [`super::plan::apply_event`]，
//! 與第一階段共用同一套「更新既有列或新增資料列」的規則；
//! 仍然判不出來的事件只會換一個原因留在待處理清單裡，絕不猜季別寫進資料庫。

use std::collections::HashMap;

use chrono::NaiveDate;
use rand::RngExt;

use crate::{
    domain::dividend::entity::Dividend,
    infra::crawler::{
        moneydj::dividend_schedule::DividendSchedule, share::ExDividendAnnouncement,
        yahoo::dividend::YahooDividend,
    },
};

use super::{
    DATE_FORMAT,
    plan::{ScanPlan, apply_event},
    quarter::{ResolvedDividend, UnresolvedEvent, UnresolvedReason},
    source::fetch_yahoo_dividend,
};

/// 單次掃描允許查詢 Yahoo 的股票檔數上限。
///
/// 正常情況下需要補救的只有個位數；設上限是為了避免資料庫剛清空或
/// 上游大量改版時，一次排程對 Yahoo 發出數百個請求而被擋。
const YAHOO_LOOKUP_LIMIT: usize = 60;

/// 第二階段：對第一階段判不出期別的事件逐檔查詢 Yahoo 股利政策。
///
/// 同一檔股票只查一次（同一天可能有多筆事件），查詢之間加 1.5～3.0 秒隨機延遲，
/// 與既有的 [`crate::app::backfill::dividend::unannounced_ex_dividend_date`] 一致，
/// 降低被 Yahoo WAF 擋下的機率。
pub(super) async fn resolve_with_yahoo(
    plan: &mut ScanPlan,
    schedules: &HashMap<(String, NaiveDate), DividendSchedule>,
    existing: &HashMap<String, Vec<Dividend>>,
) {
    let pending: Vec<UnresolvedEvent> = std::mem::take(&mut plan.unresolved);
    if pending.is_empty() {
        return;
    }

    // 逐檔快取本次查詢結果，避免同一檔股票的多筆事件重複請求。
    let mut fetched: HashMap<String, Option<YahooDividend>> = HashMap::new();
    let mut lookups = 0usize;

    for event in pending {
        let symbol = event.announcement.stock_symbol.clone();

        if !event.reason.is_retryable_with_yahoo() {
            plan.unresolved.push(event);
            continue;
        }

        if !fetched.contains_key(&symbol) {
            if lookups >= YAHOO_LOOKUP_LIMIT {
                plan.unresolved.push(UnresolvedEvent {
                    reason: UnresolvedReason::YahooLookupSkipped,
                    ..event
                });
                continue;
            }

            // 第二筆以後才需要間隔；第一筆直接查，不必平白等待。
            if lookups > 0 {
                let jitter_ms = rand::rng().random_range(1500..=3000);
                tokio::time::sleep(std::time::Duration::from_millis(jitter_ms)).await;
            }
            lookups += 1;
            fetched.insert(symbol.clone(), fetch_yahoo_dividend(&symbol).await);
        }

        let Some(Some(yahoo)) = fetched.get(&symbol) else {
            plan.unresolved.push(UnresolvedEvent {
                reason: UnresolvedReason::YahooLookupFailed,
                ..event
            });
            continue;
        };

        let Some(resolved) = resolve_from_yahoo(&event.announcement, yahoo) else {
            plan.unresolved.push(UnresolvedEvent {
                reason: UnresolvedReason::NoMatchingYahooDividend,
                ..event
            });
            continue;
        };

        apply_event(
            plan,
            &event.announcement,
            Some(&resolved),
            schedules,
            existing,
            UnresolvedReason::NoMatchingYahooDividend,
        );
    }

    tracing::info!(lookups, "Yahoo 逐檔補救查詢完成");
}

/// 從 Yahoo 的股利政策中找出除權息日相符的那一次配息。
///
/// 用日期而不是金額比對：Yahoo 的明細本身就帶除息日與除權日，
/// 日期相符是比金額吻合更強的證據，也不會被「同年兩期同額」難倒。
///
/// **必須掃過所有發放年度分組**：Yahoo 是依發放年度分組的，12 月除息、隔年 1 月發放的
/// 事件會被放進隔年，只查除權息日那一年會整個漏掉。
fn resolve_from_yahoo(
    announcement: &ExDividendAnnouncement,
    yahoo: &YahooDividend,
) -> Option<ResolvedDividend> {
    let ex_date_text = announcement.ex_date.format(DATE_FORMAT).to_string();

    yahoo
        .dividend
        .iter()
        .flat_map(|(_, details)| details.iter())
        .find(|detail| {
            (announcement.is_cash && detail.ex_dividend_date1 == ex_date_text)
                || (announcement.is_stock && detail.ex_dividend_date2 == ex_date_text)
        })
        .map(ResolvedDividend::from_yahoo)
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::super::UNANNOUNCED_DATE;
    use super::super::fixtures::{announcement, yahoo_detail};
    use super::*;
    use crate::infra::crawler::tpex;

    /// Yahoo 補救：以除權息日相符找出對應配息，取它的所屬年度、季別與發放年度。
    #[test]
    fn test_resolve_from_yahoo_matches_by_ex_date() {
        let ann = announcement("6488", (2026, 8, 26), true, false, Some(dec!(10.0)), None);
        let yahoo = YahooDividend {
            stock_symbol: "6488".to_string(),
            dividend: vec![(
                2026,
                vec![
                    yahoo_detail(
                        2026,
                        2025,
                        "H1",
                        dec!(5.0),
                        Decimal::ZERO,
                        "2026-03-10",
                        UNANNOUNCED_DATE,
                    ),
                    yahoo_detail(
                        2026,
                        2025,
                        "H2",
                        dec!(10.0),
                        Decimal::ZERO,
                        "2026-08-26",
                        UNANNOUNCED_DATE,
                    ),
                ],
            )],
        };

        let item = resolve_from_yahoo(&ann, &yahoo).expect("ex-date should match");

        assert_eq!(item.quarter, "H2");
        assert_eq!(item.year_of_dividend, 2025);
        assert_eq!(item.paid_year, Some(2026));
        assert_eq!(item.cash_dividend, dec!(10.0));
        // Yahoo 沒有拆分資料，盈餘／公積欄位必須留 0，避免高估盈餘分配率。
        assert_eq!(item.earnings_cash, Decimal::ZERO);
        assert_eq!(item.capital_reserve_cash, Decimal::ZERO);
    }

    /// 跨年發放的事件被 Yahoo 歸在隔年，只查除權息日那一年會整個漏掉。
    #[test]
    fn test_resolve_from_yahoo_searches_every_paid_year() {
        let ann = announcement("2882", (2025, 12, 20), true, false, Some(dec!(3.0)), None);
        let yahoo = YahooDividend {
            stock_symbol: "2882".to_string(),
            dividend: vec![(
                2026,
                vec![yahoo_detail(
                    2026,
                    2025,
                    "Q4",
                    dec!(3.0),
                    Decimal::ZERO,
                    "2025-12-20",
                    UNANNOUNCED_DATE,
                )],
            )],
        };

        let item = resolve_from_yahoo(&ann, &yahoo).expect("cross-year event should match");
        assert_eq!(item.paid_year, Some(2026));
    }

    /// 同一年沒有除權息日相符的紀錄時不可硬湊。
    #[test]
    fn test_resolve_from_yahoo_returns_none_without_match() {
        let ann = announcement("6488", (2026, 8, 26), true, false, Some(dec!(10.0)), None);
        let yahoo = YahooDividend {
            stock_symbol: "6488".to_string(),
            dividend: vec![(
                2026,
                vec![yahoo_detail(
                    2026,
                    2025,
                    "H1",
                    dec!(5.0),
                    Decimal::ZERO,
                    "2026-03-10",
                    UNANNOUNCED_DATE,
                )],
            )],
        };

        assert!(resolve_from_yahoo(&ann, &yahoo).is_none());
    }

    /// 純除權事件要用除權日比對，不能只看除息日。
    #[test]
    fn test_resolve_from_yahoo_matches_stock_ex_date() {
        let ann = announcement("3234", (2026, 8, 26), false, true, None, Some(dec!(0.1)));
        let yahoo = YahooDividend {
            stock_symbol: "3234".to_string(),
            dividend: vec![(
                2026,
                vec![yahoo_detail(
                    2026,
                    2025,
                    "",
                    Decimal::ZERO,
                    dec!(1.0),
                    UNANNOUNCED_DATE,
                    "2026-08-26",
                )],
            )],
        };

        let item = resolve_from_yahoo(&ann, &yahoo).expect("stock ex-date should match");
        assert_eq!(item.stock_dividend, dec!(1.0));
    }

    /// 對真實的上櫃／ETF 事件實跑第二階段，確認 Yahoo 能補出期別。
    ///
    /// 只查前幾檔以免測試時間過長；同樣不寫入資料庫。
    #[tokio::test]
    #[ignore]
    async fn test_yahoo_fallback_against_live_site() {
        dotenvy::dotenv().ok();

        let otc = tpex::ex_dividend_announcement::visit()
            .await
            .expect("上櫃預告表應可取得");
        let sample: Vec<ExDividendAnnouncement> = otc.into_iter().take(3).collect();

        for announcement in sample {
            let Some(yahoo) = fetch_yahoo_dividend(&announcement.stock_symbol).await else {
                println!("{} 抓取失敗", announcement.stock_symbol);
                continue;
            };

            match resolve_from_yahoo(&announcement, &yahoo) {
                Some(item) => println!(
                    "{} {} → 發放年度 {:?} {}{} 現金 {} 股票 {}",
                    announcement.stock_symbol,
                    announcement.ex_date,
                    item.paid_year,
                    item.year_of_dividend,
                    if item.quarter.is_empty() {
                        "年度".to_string()
                    } else {
                        item.quarter.clone()
                    },
                    item.cash_dividend,
                    item.stock_dividend
                ),
                None => println!(
                    "{} {} 找不到除權息日相符的 Yahoo 紀錄",
                    announcement.stock_symbol, announcement.ex_date
                ),
            }

            tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
        }
    }
}
