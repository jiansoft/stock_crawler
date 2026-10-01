use std::time::Duration;

use crate::{
    app::backfill::acl::{QfiiAclMapper, UpdateQfiiCommand},
    core::util::datetime::Weekend,
    domain::events::DomainEvent,
    domain::foreign_holding::{entity::ForeignHolding, repository::ForeignHoldingRepository},
    domain::registry::repository::StockRepository,
    infra::crawler::{share::QfiiDto, twse},
    infra::database::repository::{
        foreign_holding::PgForeignHoldingRepository, stock::PgStockRepository,
    },
};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Datelike, Days, FixedOffset, Local, NaiveDate, Weekday};
use scopeguard::defer;

/// 回補歷史時每個交易日之間的間隔：證交所對短時間大量請求會暫時封鎖。
const HISTORY_BACKFILL_INTERVAL: Duration = Duration::from_secs(3);

/// 回補上市與上櫃外資持股狀況。
///
/// 更新股票主檔的最新持股後，再把當天快照寫進 `qfii_history`、重算 `qfii_trend`，
/// 最後派發 [`DomainEvent::ForeignHoldingsUpdated`] 讓持股通知檢查外資明顯增減持的股票。
pub async fn execute() -> Result<()> {
    let now = Local::now();

    if now.is_weekend() {
        return Ok(());
    }
    tracing::info!("更新台股外資持股狀態開始");
    defer! {
       tracing::info!("更新台股外資持股狀態結束");
    }

    let today = now.date_naive();
    let (listed_rows, otc) = tokio::try_join!(listed(now.fixed_offset()), otc())?;

    // 上市以查詢日為資料日；休市日沒有資料（空清單）。上櫃以櫃買回應的日期為準，
    // 休市日會回最近一個交易日的資料，重寫一次不會有變更。
    let mut history = to_history(&listed_rows, today);
    history.extend(to_history(&otc.rows, otc.date));

    let repo = PgForeignHoldingRepository::new();
    let changed = repo.save_daily(&history).await?;
    tracing::info!(
        "外資持股歷史寫入 {} 筆（變更 {} 筆）",
        history.len(),
        changed
    );
    if changed == 0 {
        // 沒有新資料（休市日、同一天重跑）時不重算也不通知，避免重複推播。
        return Ok(());
    }

    let as_of = if listed_rows.is_empty() {
        otc.date
    } else {
        today.max(otc.date)
    };
    let trends = repo.rebuild_trends_as_of(as_of).await?;
    tracing::info!("外資持股趨勢重算 {} 檔（基準日 {}）", trends, as_of);

    crate::app::event::get_global_dispatcher()
        .dispatch_async(vec![DomainEvent::ForeignHoldingsUpdated {
            date: as_of,
            occurred_at: Local::now(),
        }])
        .await;

    Ok(())
}

async fn listed(date_time: DateTime<FixedOffset>) -> Result<Vec<QfiiDto>> {
    let listed = twse::qualified_foreign_institutional_investor::listed::visit(date_time).await?;
    // 記下筆數：上櫃曾因來源頁 404 而默默更新 0 筆，日誌裡看不出任何異狀。
    tracing::info!("上市外資持股取得 {} 筆", listed.len());
    let cmds = listed.iter().map(QfiiAclMapper::from_qfii).collect();
    update(cmds).await?;
    Ok(listed)
}

/// 回補上櫃外資持股資料。
async fn otc()
-> Result<twse::qualified_foreign_institutional_investor::over_the_counter::OtcQfiiSnapshot> {
    let snapshot =
        twse::qualified_foreign_institutional_investor::over_the_counter::visit().await?;
    tracing::info!(
        "上櫃外資持股取得 {} 筆（資料日 {}）",
        snapshot.rows.len(),
        snapshot.date
    );
    let cmds = snapshot.rows.iter().map(QfiiAclMapper::from_qfii).collect();
    update(cmds).await?;
    Ok(snapshot)
}

/// 將爬蟲資料轉成指定資料日的外資持股快照。
fn to_history(rows: &[QfiiDto], date: NaiveDate) -> Vec<ForeignHolding> {
    rows.iter()
        .map(|row| ForeignHolding {
            stock_symbol: row.stock_symbol.clone(),
            date,
            issued_share: row.issued_share,
            shares_held: row.shares_held,
            share_holding_percentage: row.share_holding_percentage,
        })
        .collect()
}

/// 外資持股歷史回補結果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ForeignHoldingHistoryBackfillSummary {
    /// 有資料的交易日數。
    pub trading_days: usize,
    /// 寫入（新增或變更）的筆數。
    pub saved_rows: u64,
    /// 最後重算趨勢的檔數。
    pub trend_rows: u64,
}

/// 回補 `[start, end]` 期間每個交易日的外資持股歷史，最後以最後一個交易日重算趨勢。
///
/// 逐日抓證交所 `MI_QFIIS` 與櫃買 QFII API；週末直接略過，平日休市時兩邊都沒有資料。
/// 回補不派發通知事件——歷史資料不是「剛發生的變化」，推播只會造成一次大量洗版。
pub async fn backfill_history(
    start: NaiveDate,
    end: NaiveDate,
) -> Result<ForeignHoldingHistoryBackfillSummary> {
    if start > end {
        return Err(anyhow!("backfill start {start} is after end {end}"));
    }

    let repo = PgForeignHoldingRepository::new();
    let offset = FixedOffset::east_opt(8 * 3600).context("invalid Taipei offset")?;
    let mut summary = ForeignHoldingHistoryBackfillSummary::default();
    let mut last_trading_day = None;
    let mut day = start;

    while day <= end {
        if !matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
            let date_time = day
                .and_hms_opt(15, 0, 0)
                .and_then(|naive| naive.and_local_timezone(offset).single())
                .with_context(|| format!("invalid date {day}"))?;
            let listed =
                twse::qualified_foreign_institutional_investor::listed::visit(date_time).await?;
            let otc =
                twse::qualified_foreign_institutional_investor::over_the_counter::visit_on(day)
                    .await?;

            let mut history = to_history(&listed, day);
            // 櫃買指定日期時回的就是該日；保險起見只收日期相符的資料。
            if otc.date == day {
                history.extend(to_history(&otc.rows, day));
            }

            if !history.is_empty() {
                summary.saved_rows += repo.save_daily(&history).await?;
                summary.trading_days += 1;
                last_trading_day = Some(day);
                tracing::info!(
                    "外資持股歷史回補 {}：上市 {} 筆、上櫃 {} 筆",
                    day,
                    listed.len(),
                    otc.rows.len()
                );
            }

            tokio::time::sleep(HISTORY_BACKFILL_INTERVAL).await;
        }

        day = day
            .checked_add_days(Days::new(1))
            .with_context(|| format!("date overflow after {day}"))?;
    }

    if let Some(as_of) = last_trading_day {
        summary.trend_rows = repo.rebuild_trends_as_of(as_of).await?;
    }

    Ok(summary)
}

/// 更新股票的外資持股狀況，資料庫更新後會更新 SHARE.stocks
async fn update(cmds: Vec<UpdateQfiiCommand>) -> Result<()> {
    let repo = PgStockRepository::new();

    for cmd in cmds {
        // 嘗試讀取 Stock 聚合根
        let stock_opt = repo.find_by_symbol(&cmd.symbol).await?;
        if let Some(mut stock) = stock_opt {
            if stock.issued_share() == cmd.issued_share
                && stock.qfii_shares_held() == cmd.shares_held
                && stock.qfii_share_holding_percentage() == cmd.share_holding_percentage
            {
                continue;
            }

            // 使用領域模型更新狀態
            stock.update_qfii(cmd.shares_held, cmd.share_holding_percentage);
            stock.update_issued_shares(cmd.issued_share);

            // 儲存 Stock 聚合根，同時更新 DB 與快取
            if let Err(why) = repo.save(&stock).await {
                tracing::error!(
                    "Failed to save stock QFII updates for {} because {:?}",
                    cmd.symbol,
                    why
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use crate::infra::cache::SHARE;

    use super::*;

    #[test]
    fn to_history_stamps_every_row_with_the_data_date() {
        let rows = vec![QfiiDto {
            stock_symbol: "2330".to_string(),
            issued_share: 25_932_370_067,
            shares_held: 17_951_023_616,
            share_holding_percentage: dec!(69.22),
        }];
        let date = NaiveDate::from_ymd_opt(2026, 8, 28).unwrap();

        let history = to_history(&rows, date);

        assert_eq!(
            history,
            vec![ForeignHolding {
                stock_symbol: "2330".to_string(),
                date,
                issued_share: 25_932_370_067,
                shares_held: 17_951_023_616,
                share_holding_percentage: dec!(69.22),
            }]
        );
    }

    #[tokio::test]
    async fn backfill_history_rejects_reversed_range() {
        let start = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        assert!(backfill_history(start, end).await.is_err());
    }

    /// 驗證外資持股回補流程。
    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 execute");

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to execute because {:?}", why);
            }
        }

        tracing::debug!("結束 execute");
    }
}
