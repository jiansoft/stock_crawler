use crate::{
    app::backfill,
    app::calculation,
    core::{alert, util::datetime::Weekend},
    domain::quote::{
        after_hours::AfterHoursDataQuery,
        entity::MarketTradedCount,
        forward_return::{self, DerivedCounts, DerivedDataQuery, ForwardReturnRepository},
        repository::QuoteRepository,
    },
    infra::cache::{TTL, TtlCacheInner},
    infra::crawler::{self, twse},
    infra::database::repository::{quote::PgQuoteRepository, yield_rank::PgYieldRankRepository},
};
use anyhow::Result;
use chrono::{DateTime, Datelike, Local, NaiveDate};
use scopeguard::defer;

mod after_hours;

pub use after_hours::ensure_after_hours;
mod derived_check;
mod quote_check;

// 行情完整性與衍生資料檢查搬到子模組；排程入口與收盤匯總留在這裡。
use self::{derived_check::*, quote_check::*};
// 資料健康週報也用這個判斷交易日缺漏。
pub(crate) use self::quote_check::shortfalls;

/// 台股收盤事件發生時要進行的事情
pub async fn execute() -> Result<()> {
    tracing::info!("台股收盤事件開始");
    defer! {
       tracing::info!("台股收盤事件結束");
    }

    let now = Local::now();
    let current_date: NaiveDate = now.date_naive();
    if is_market_closed(now).await {
        tracing::info!("{current_date} 休市，略過收盤匯總");
        return Ok(());
    }

    let aggregate = aggregate(current_date);
    let index = backfill::taiwan_stock_index::execute();
    let (res_aggregation, res_index) = tokio::join!(aggregate, index);

    if let Err(why) = res_index {
        tracing::error!("Failed to taiwan_stock_index::execute() because {:#?}", why);
    }

    if let Err(why) = res_aggregation {
        tracing::error!("Failed to closing::aggregate() because {:#?}", why);
    }

    // 停止 trace 事件所使用的即時報價背景任務
    crate::app::event::trace::price_tasks::stop_price_tasks().await;

    crawler::flush_site_latency_stats();

    Ok(())
}

/// 排程入口：檢查今天的收盤資料是否完整，不完整就重跑收盤匯總並告警。
///
/// 15:00 的收盤匯總可能因來源尚未公布或暫時失敗而沒有寫入，或只有一個市場有資料；
/// 舊版沒有任何重試，該日就一直缺著，「缺漏補齊」還會把缺的市場補成整天零量列
/// （2018～2026 年共找到 15 個這樣的交易日）。這裡在盤後再檢查：上市或上櫃有成交的檔數
/// 低於近期交易日的 [`COVERAGE_MIN_PERCENT`]% 就重跑 [`aggregate`]，並把結果告警出來。
///
/// 行情檢查之後再以 [`ensure_derived`] 檢查技術訊號與未來報酬，最後檢查同時段寫入的大盤指數
/// （見 [`after_hours`]）。
///
/// # Errors
///
/// 查詢有成交檔數或衍生資料筆數失敗時回傳錯誤；重跑失敗只告警，不回傳錯誤。
pub async fn ensure_complete() -> Result<()> {
    let now = Local::now();
    let date = now.date_naive();
    if is_market_closed(now).await {
        return Ok(());
    }

    if ensure_quotes(date, || aggregate(date)).await? == QuoteCheck::UnscheduledClosure {
        return Ok(());
    }
    ensure_derived(date).await?;
    after_hours::ensure(date, &after_hours::WITH_CLOSING).await
}

/// 週末或交易所公告的休市日回傳 `true`。
///
/// 休市日 TWSE `MI_INDEX` 只回 `{"stat":"很抱歉，沒有符合條件的資料!"}`，缺少 `tables`
/// 讓解析失敗，每逢週末、假日都記一筆 error（2026-09-25～28 連續四天）。
/// 休市日清單抓取失敗時視為開市，交給 [`aggregate`] 原本「0 筆即略過」的邏輯處理。
async fn is_market_closed(now: DateTime<Local>) -> bool {
    if now.is_weekend() {
        return true;
    }

    let today = now.date_naive();
    match twse::holiday_schedule::visit(today.year()).await {
        Ok(holidays) => holidays.iter().any(|holiday| holiday.date == today),
        Err(why) => {
            tracing::warn!(
                "Failed to fetch TWSE holiday schedule, continuing closing aggregation: {:?}",
                why
            );
            false
        }
    }
}

/// 股票收盤數據匯總。
///
/// 此函式會串起收盤資料回補、缺漏報價補齊、均線、最後交易日報價、
/// 估價、殖利率排行、市值重算與市值變化通知。主要由 [`execute`] 呼叫，
/// 測試環境也會透過手動回補測試檔指定日期執行。
///
/// # Errors
///
/// 任一步驟失敗時會回傳錯誤，呼叫端可依情境記錄或中止後續流程。
pub(crate) async fn aggregate(date: NaiveDate) -> Result<()> {
    //抓取上市櫃公司每日收盤資訊
    let daily_quote_count = backfill::quote::execute(date).await?;
    tracing::info!("抓取上市櫃收盤數據結束:{}", daily_quote_count);

    if daily_quote_count == 0 {
        // 非交易日（假日/收盤 0 筆）：完全略過本次收盤匯總，包含帳戶市值計算，
        // 避免 daily_money_history 系列表被寫入非交易日（如週末）的無效紀錄
        // （部分表格的 upsert SQL 以「當日收盤價精確比對」為前提，非交易日
        // 沒有對應收盤價時會產生全 0 的錯誤資料列）
        tracing::info!("日期 {} 無交易資料，略過本次收盤匯總", date);
        return Ok(());
    }

    // 除權息日的漲跌在交易所行情中是 0，要在均線、統計表與排行之前以官方基準價重算；
    // 失敗只影響當天除權息股的漲跌，不中斷收盤匯總
    match backfill::ex_right_change::execute(date).await {
        Ok(summary) => tracing::info!("除權息日漲跌修正結束:{summary:?}"),
        Err(why) => tracing::error!("除權息日漲跌修正失敗: {why:#}"),
    }

    // 有行情資料才執行的計算（均線、估價、殖利率排行）
    let quote_repo = PgQuoteRepository::new();

    let lack_daily_quotes_count = quote_repo.makeup_for_the_lack_daily_quotes(date).await?;
    tracing::info!(
        "補上當日缺少的每日收盤數據結束:{:#?}",
        lack_daily_quotes_count
    );

    calculation::daily_quotes::calculate_moving_average(date).await?;
    tracing::info!("計算均線結束");

    quote_repo.rebuild_last_daily_quotes().await?;
    tracing::info!("重建 last_daily_quotes 表內的數據結束");

    calculation::estimated_price::calculate_estimated_price(date).await?;
    tracing::info!("計算便宜、合理、昂貴價的估算結束");

    let yield_rank_repo = PgYieldRankRepository::new();
    use crate::domain::yield_rank::repository::YieldRankRepository;
    yield_rank_repo.rebuild_by_date(date).await?;
    tracing::info!("重建 yield_rank 表內的數據結束");

    calculation::money_history::calculate_money_history(date).await?;
    tracing::info!("計算帳戶內市值結束");
    // 技術訊號只供查詢與通知使用，失敗不影響後續的市值變化通知
    match calculation::technical_signal::calculate_technical_signals(date).await {
        Ok(count) => tracing::info!("計算技術訊號結束:{count}"),
        Err(why) => tracing::error!("計算技術訊號失敗: {why:#}"),
    }
    // 未來報酬只供訊號績效統計使用，失敗不影響後續流程
    match calculation::forward_return::calculate_forward_returns(date).await {
        Ok(count) => tracing::info!("回填未來報酬結束:{count}"),
        Err(why) => tracing::error!("回填未來報酬失敗: {why:#}"),
    }
    // 相對強度要用到剛回填的未來報酬；失敗只記錄，19:00 完整性檢查會補算
    match calculation::relative_strength::calculate_relative_strength(date).await {
        Ok(count) => tracing::info!("寫入相對強度結束:{count}"),
        Err(why) => tracing::error!("寫入相對強度失敗: {why:#}"),
    }

    // 清除記憶與Redis內所有的快取
    TTL.clear();

    // 派發領域事件以非同步處理本日與前一個交易日的市值變化通知
    let dispatcher = crate::app::event::get_global_dispatcher();
    dispatcher
        .dispatch_async(vec![
            crate::domain::events::DomainEvent::MoneyFlowRecalculated {
                date,
                occurred_at: chrono::Local::now(),
            },
        ])
        .await;

    Ok(())
}

#[cfg(test)]
mod tests;
