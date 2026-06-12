use crate::{
    app::backfill,
    app::calculation,
    core::logging,
    domain::quote::repository::QuoteRepository,
    infra::cache::{TTL, TtlCacheInner},
    infra::crawler,
    infra::database::repository::{quote::PgQuoteRepository, yield_rank::PgYieldRankRepository},
};
use anyhow::Result;
use chrono::{Local, NaiveDate};
use scopeguard::defer;

/// 台股收盤事件發生時要進行的事情
pub async fn execute() -> Result<()> {
    logging::info_file_async("台股收盤事件開始");
    defer! {
       logging::info_file_async("台股收盤事件結束");
    }

    let current_date: NaiveDate = Local::now().date_naive();
    let aggregate = aggregate(current_date);
    let index = backfill::taiwan_stock_index::execute();
    let (res_aggregation, res_index) = tokio::join!(aggregate, index);

    if let Err(why) = res_index {
        logging::error_file_async(format!(
            "Failed to taiwan_stock_index::execute() because {:#?}",
            why
        ));
    }

    if let Err(why) = res_aggregation {
        logging::error_file_async(format!("Failed to closing::aggregate() because {:#?}", why));
    }

    // 停止 trace 事件所使用的即時報價背景任務
    crate::app::event::trace::price_tasks::stop_price_tasks().await;

    crawler::flush_site_latency_stats();

    Ok(())
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
    //logging::info_file_async("抓取上市櫃收盤數據結束".to_string());
    //let daily_quote_count = daily_quote::fetch_count_by_date(date).await?;
    logging::info_file_async(format!("抓取上市櫃收盤數據結束:{}", daily_quote_count));

    if daily_quote_count == 0 {
        return Ok(());
    }

    // 實例化報價領域的倉儲
    let quote_repo = PgQuoteRepository::new();

    // 補上當日缺少的每日收盤數據
    let lack_daily_quotes_count = quote_repo.makeup_for_the_lack_daily_quotes(date).await?;
    logging::info_file_async(format!(
        "補上當日缺少的每日收盤數據結束:{:#?}",
        lack_daily_quotes_count
    ));

    // 計算均線
    calculation::daily_quotes::calculate_moving_average(date).await?;
    logging::info_file_async("計算均線結束".to_string());

    // 重建 last_daily_quotes 表內的數據
    quote_repo.rebuild_last_daily_quotes().await?;
    logging::info_file_async("重建 last_daily_quotes 表內的數據結束".to_string());

    // 計算便宜、合理、昂貴價的估算
    calculation::estimated_price::calculate_estimated_price(date).await?;
    logging::info_file_async("計算便宜、合理、昂貴價的估算結束".to_string());

    // 實例化殖利率排行領域的倉儲
    let yield_rank_repo = PgYieldRankRepository::new();
    use crate::domain::yield_rank::repository::YieldRankRepository;
    // 重建指定日期的 yield_rank 表內的數據
    yield_rank_repo.rebuild_by_date(date).await?;
    logging::info_file_async("重建 yield_rank 表內的數據結束".to_string());

    // 計算帳戶內市值
    calculation::money_history::calculate_money_history(date).await?;
    logging::info_file_async("計算帳戶內市值結束".to_string());

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
mod tests {
    use super::*;
    use crate::{core::logging, infra::cache::SHARE};
    use std::time::Duration;

    /// 每日收盤事件主要匯總流程的整合測試。
    #[tokio::test]
    #[ignore]
    async fn test_aggregate() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        logging::debug_file_async("開始 event::taiwan_stock::closing::aggregate".to_string());

        let current_date = NaiveDate::parse_from_str("2026-04-30", "%Y-%m-%d").unwrap();

        match aggregate(current_date).await {
            Ok(_) => {
                logging::debug_file_async(
                    "event::taiwan_stock::closing::aggregate 完成".to_string(),
                );
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to event::taiwan_stock::closing::aggregate because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 event::taiwan_stock::closing::aggregate".to_string());
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
