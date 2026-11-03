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
    let sources = YahooSources {
        fetch: |symbol: String| async move { fetch_yahoo_dividend(&symbol).await },
        pause: || {
            let jitter_ms = rand::rng().random_range(1500..=3000);
            tokio::time::sleep(std::time::Duration::from_millis(jitter_ms))
        },
        limit: YAHOO_LOOKUP_LIMIT,
    };
    resolve_with(plan, schedules, existing, sources).await;
}

/// 第二階段的外部依賴：逐檔抓 Yahoo、兩次查詢之間的間隔，與單次掃描的查詢上限。
struct YahooSources<Fetch, Pause> {
    fetch: Fetch,
    pause: Pause,
    limit: usize,
}

/// [`resolve_with_yahoo`] 的流程本體；Yahoo 來源與間隔由呼叫端注入。
async fn resolve_with<Fetch, FetchFut, Pause, PauseFut>(
    plan: &mut ScanPlan,
    schedules: &HashMap<(String, NaiveDate), DividendSchedule>,
    existing: &HashMap<String, Vec<Dividend>>,
    mut sources: YahooSources<Fetch, Pause>,
) where
    Fetch: FnMut(String) -> FetchFut,
    FetchFut: Future<Output = Option<YahooDividend>>,
    Pause: FnMut() -> PauseFut,
    PauseFut: Future<Output = ()>,
{
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
            if lookups >= sources.limit {
                plan.unresolved.push(UnresolvedEvent {
                    reason: UnresolvedReason::YahooLookupSkipped,
                    ..event
                });
                continue;
            }

            // 第二筆以後才需要間隔；第一筆直接查，不必平白等待。
            if lookups > 0 {
                (sources.pause)().await;
            }
            lookups += 1;
            let yahoo = (sources.fetch)(symbol.clone()).await;
            fetched.insert(symbol.clone(), yahoo);
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
mod tests;
