//! 盤後資料完整性檢查：大盤指數、三大法人與融資融券、外資持股與官方本益比。
//!
//! 這些資料由各自的排程在收盤後寫入（指數 15:00、官方本益比 17:30／20:30、籌碼 21:50、外資持股
//! 22:00），排程失敗（例如資料庫停機）時當天就一直缺著。這裡在當天收盤匯總之後再檢查一次：
//! 指數在 17:00／19:00 的收盤檢查一起看，其餘在 23:00 檢查；筆數為 0 或低於近期交易日最大值的
//! [`COVERAGE_MIN_PERCENT`]% 就重跑對應的排程並告警。官方本益比只會補日線為 0 的欄位、重跑安全，
//! 筆數也看不出有沒有漏補，所以直接重跑，有補到才告警。

use anyhow::Result;
use chrono::{Local, NaiveDate};

use super::{COVERAGE_MIN_PERCENT, is_market_closed};
use crate::{
    app::backfill::taiwan_stock_index,
    app::backfill::{chip, official_ratio, qualified_foreign_institutional_investor},
    core::alert,
    domain::quote::{
        after_hours::{AfterHoursCounts, AfterHoursDataQuery},
        forward_return::ForwardReturnRepository,
    },
    infra::database::repository::{chip::PgChipRepository, quote::PgQuoteRepository},
};

/// 比較的近期交易日數（含當天）。
const LOOKBACK_TRADING_DAYS: usize = 6;

/// 檢查的盤後資料。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AfterHours {
    /// 加權指數（`index` 表 `TAIEX`）。
    Index,
    /// 三大法人與融資融券（`chip_daily`）。
    Chip,
    /// 外資持股（`qfii_history`）。
    ForeignHolding,
}

impl AfterHours {
    fn label(self) -> &'static str {
        match self {
            Self::Index => "大盤指數",
            Self::Chip => "三大法人與融資融券",
            Self::ForeignHolding => "外資持股",
        }
    }

    fn count(self, counts: &AfterHoursCounts) -> i64 {
        match self {
            Self::Index => counts.index,
            Self::Chip => counts.chip,
            Self::ForeignHolding => counts.foreign_holding,
        }
    }
}

/// 收盤檢查（17:00／19:00）一起看的盤後資料。
pub(crate) const WITH_CLOSING: [AfterHours; 1] = [AfterHours::Index];

/// 23:00 檢查的盤後資料（指數再看一次）。
pub(crate) const LATE_EVENING: [AfterHours; 3] = [
    AfterHours::Index,
    AfterHours::Chip,
    AfterHours::ForeignHolding,
];

/// 排程入口（23:00）：檢查指數、籌碼與外資持股，缺漏就重跑並告警；再重跑一次官方本益比補寫。
///
/// # Errors
///
/// 查詢交易日或筆數失敗時回傳錯誤；重跑失敗只告警，不回傳錯誤。
pub async fn ensure_after_hours() -> Result<()> {
    let now = Local::now();
    if is_market_closed(now).await {
        return Ok(());
    }
    let date = now.date_naive();
    ensure(date, &LATE_EVENING).await?;
    ensure_official_ratios(date).await;
    Ok(())
}

/// 檢查 `items` 並在缺漏時重跑對應的排程、發告警。
pub(crate) async fn ensure(date: NaiveDate, items: &[AfterHours]) -> Result<()> {
    let report = check_after_hours(&PgQuoteRepository::new(), date, items, |item| async move {
        match item {
            AfterHours::Index => taiwan_stock_index::execute_for_date(date).await.map(|_| ()),
            AfterHours::Chip => chip::save_daily(&PgChipRepository::new(), date)
                .await
                .map(|_| ()),
            // 外資持股排程只抓當天，`date` 一定是今天（見 `ensure_after_hours`）。
            AfterHours::ForeignHolding => qualified_foreign_institutional_investor::execute().await,
        }
    })
    .await?;
    if let Some((title, body)) = report {
        alert::send_alert(title, &body).await;
    }
    Ok(())
}

/// [`ensure`] 的本體：檢查、以 `recompute` 重跑缺的項目，回傳要發的告警（標題、內容）；不需要告警時
/// 回 `None`。當天沒有日線（休市、收盤尚未寫入）時不檢查。倉儲與重跑可替換以便測試。
pub(crate) async fn check_after_hours<R, F, Fut>(
    repo: &R,
    date: NaiveDate,
    items: &[AfterHours],
    recompute: F,
) -> Result<Option<(&'static str, String)>>
where
    R: ForwardReturnRepository + AfterHoursDataQuery,
    F: Fn(AfterHours) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let dates = repo
        .fetch_trading_dates(date, LOOKBACK_TRADING_DAYS)
        .await?;
    if dates.first() != Some(&date) {
        return Ok(None);
    }
    let before = missing_after_hours(items, &repo.count_after_hours(&dates).await?);
    if before.is_empty() {
        tracing::info!("{date} 盤後資料完整: {items:?}");
        return Ok(None);
    }

    tracing::warn!("{date} 盤後資料缺漏，重跑: {before:?}");
    let mut errors = Vec::new();
    for item in &before {
        if let Err(why) = recompute(*item).await {
            tracing::error!("{date} 重跑{}失敗: {why:#}", item.label());
            errors.push(format!("{}：{why:#}", item.label()));
        }
    }
    let after = missing_after_hours(items, &repo.count_after_hours(&dates).await?);
    let title = if after.is_empty() {
        "盤後資料已補齊"
    } else {
        "盤後資料仍缺漏"
    };
    Ok(Some((title, report(date, &before, &errors, &after))))
}

/// `counts` 第一筆（當天）筆數為 0，或低於之後幾天（近期交易日）最大值的 [`COVERAGE_MIN_PERCENT`]%
/// 的項目。
pub(crate) fn missing_after_hours(
    items: &[AfterHours],
    counts: &[AfterHoursCounts],
) -> Vec<AfterHours> {
    let Some((today, recent)) = counts.split_first() else {
        return Vec::new();
    };
    items
        .iter()
        .copied()
        .filter(|item| {
            let count = item.count(today);
            let baseline = recent.iter().map(|day| item.count(day)).max().unwrap_or(0);
            count == 0 || count * 100 < baseline * COVERAGE_MIN_PERCENT
        })
        .collect()
}

/// 告警內容：缺哪些、重跑失敗的原因、重跑後的狀況。
fn report(
    date: NaiveDate,
    before: &[AfterHours],
    errors: &[String],
    after: &[AfterHours],
) -> String {
    let names = |items: &[AfterHours]| {
        items
            .iter()
            .map(|item| item.label())
            .collect::<Vec<_>>()
            .join("、")
    };
    let mut lines = vec![format!("{date} 檢查時缺：{}", names(before))];
    if errors.is_empty() {
        lines.push("已重跑".to_owned());
    } else {
        lines.push(format!("重跑失敗：{}", errors.join("；")));
    }
    lines.push(if after.is_empty() {
        "重跑後都已寫入".to_owned()
    } else {
        format!(
            "重跑後仍缺：{}；請確認來源是否已公布，必要時手動回補",
            names(after)
        )
    });
    lines.join("\n")
}

/// 重跑當天的官方本益比與股價淨值比補寫；有補到資料（代表 17:30／20:30 的排程漏補）才告警。
async fn ensure_official_ratios(date: NaiveDate) {
    match official_ratio::backfill_range(date, date).await {
        Ok(summary) => {
            if let Some(body) = official_ratio_report(date, &summary) {
                alert::send_alert("官方本益比已補寫", &body).await;
            } else {
                tracing::info!("{date} 官方本益比不需補寫: {summary:?}");
            }
        }
        Err(why) => tracing::warn!("{date} 官方本益比補寫檢查失敗: {why:#}"),
    }
}

/// 有補到日線時的告警內容；沒有補到回 `None`。
pub(crate) fn official_ratio_report(
    date: NaiveDate,
    summary: &official_ratio::Summary,
) -> Option<String> {
    (summary.updated_rows > 0).then(|| {
        format!(
            "{date} 盤後檢查補上 {} 筆日線的本益比／股價淨值比（官方 {} 筆、抓取失敗 {} 個市場）",
            summary.updated_rows, summary.fetched_rows, summary.failed
        )
    })
}

#[cfg(test)]
mod tests;
