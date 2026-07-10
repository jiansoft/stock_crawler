use std::{future::Future, time::Duration};

use anyhow::{Context, Result};
use chrono::NaiveDate;
use futures::{StreamExt, stream};

use crate::{
    app::backfill::acl::QuoteAclMapper,
    core::util::{self, map::Keyable},
    domain::quote::repository::QuoteRepository,
    infra::cache::{SHARE, TTL, TtlCacheInner},
    infra::crawler::{share::DailyQuoteDto, tpex, twse},
};

/// 調用 twse、tpex API 取得台股收盤報價，並以「原子替換」寫入資料庫。
///
/// 整體流程刻意設計成「先抓取、後寫入」，順序如下：
///
/// 1. 先向 TWSE（上市）與 TPEx（上櫃）抓取當日全部收盤資料。這一步完全不動資料庫，
///    任一來源失敗就直接回傳錯誤，資料庫維持原狀。
/// 2. 若兩個來源都成功但合計 0 筆（例如假日、非交易日），也直接結束，不動資料庫。
/// 3. 抓到資料後才呼叫 [`process_quotes`]，在單一 transaction 內
///    「刪除同日舊資料 + 寫入新資料」，確保任何失敗都不會留下資料缺口。
/// 4. 最後更新「最後收盤日」系統設定。
///
/// 這個順序修正了舊版「先刪除、再抓取」的缺陷：舊版若在抓取或寫入階段失敗，
/// 已刪除的資料無法復原，該交易日的行情會永久遺失。
pub async fn execute(date: NaiveDate) -> Result<usize> {
    // 步驟 1：並行抓取上市與上櫃報價（tokio::join! 讓兩個請求同時進行以節省時間）。
    // 注意：此時尚未動到資料庫任何一筆資料。
    let twse = get_quotes_from_source(twse::quote::visit(date), "上市");
    let tpex = get_quotes_from_source(tpex::quote::visit(date), "上櫃");
    let (result_twse, result_tpex) = tokio::join!(twse, tpex);

    // 任一來源抓取失敗都立即中止：寧可整批失敗讓呼叫端重試，
    // 也不要只寫入「半個市場」的資料（例如只有上櫃、缺上市），造成資料不完整。
    let mut quotes = result_twse?;
    quotes.append(&mut result_tpex?);

    let quotes_len = quotes.len();

    // 步驟 2：0 筆代表當天不是交易日（假日）或來源尚無資料，
    // 直接結束並回報 0，既有資料原封不動。
    if quotes_len == 0 {
        return Ok(0);
    }

    // 步驟 3：有資料才進行「原子替換」寫入與快取更新；失敗會往上傳遞，
    // 由呼叫端（排程或手動回補 job）記錄錯誤，資料庫則已自動 rollback。
    process_quotes(date, quotes).await?;

    // 步驟 4：寫入成功後，更新「最後收盤日」系統設定，
    // 供其他流程（例如快取載入）得知最近一個有收盤資料的交易日。
    // 實例化系統設定領域倉儲，用來查詢與更新最後收盤日設定
    let config_repo = crate::infra::database::repository::config::PgConfigRepository::new();
    use crate::domain::config::entity::SystemConfig;
    use crate::domain::config::repository::ConfigRepository;

    // 取得資料庫中現存的最後收盤日設定
    let config_opt = config_repo.find_by_key("last-closing-day").await?;
    let should_save = match &config_opt {
        Some(cfg) => cfg.should_update_date(date),
        None => true, // 若無設定則必須寫入
    };

    // 只有在需要更新時（新日期大於已存在日期，或尚無設定時）才儲存
    if should_save {
        let new_config = SystemConfig::new(
            "last-closing-day".to_string(),
            date.format("%Y-%m-%d").to_string(),
        );
        config_repo.save(&new_config).await?;
    }
    tracing::info!("最後收盤日設定更新到資料庫完成");

    Ok(quotes_len)
}

/// 從指定資料來源抓取收盤價。
///
/// `source` 是一個尚未執行完的抓取 future（TWSE 或 TPEx），`source_name`
/// 只用於 log 與錯誤訊息。來源失敗時會附上來源名稱回傳錯誤，
/// 讓呼叫端能一眼看出是「上市」還是「上櫃」抓取失敗。
pub async fn get_quotes_from_source(
    source: impl Future<Output = Result<Vec<DailyQuoteDto>>>,
    source_name: &str,
) -> Result<Vec<DailyQuoteDto>> {
    // 舊版在這裡用 if let Ok(...) 靜默吞掉錯誤，導致來源失敗時
    // 呼叫端誤以為「當天沒有資料」；現在改為把錯誤加上說明後往上傳遞。
    let quotes = source
        .await
        .with_context(|| format!("抓取{source_name}收盤數據失敗"))?;
    tracing::info!("取完{}收盤數據: {} 筆", source_name, quotes.len());
    Ok(quotes)
}

/// 將收盤價整批「原子替換」寫入資料庫並更新主快取。
///
/// 寫入採用 [`QuoteRepository::replace_quotes_by_date`]：在同一個 transaction 內
/// 先刪除 `date` 當日既有報價、再以 COPY 批次寫入新報價，兩步同生共死。
/// 只有寫入成功後才會更新記憶體快取，避免快取與資料庫內容不一致。
pub async fn process_quotes(date: NaiveDate, quotes: Vec<DailyQuoteDto>) -> Result<usize> {
    // 將 DTO 轉換為指令對象
    let cmds: Vec<_> = quotes.iter().map(QuoteAclMapper::from_dto).collect();
    // 直接將指令對象轉換為領域層的每日報價實體
    let domain_entities: Vec<crate::domain::quote::entity::DailyQuote> =
        cmds.iter().map(QuoteAclMapper::from_command).collect();

    // 實例化報價領域的倉儲
    let repo = crate::infra::database::repository::quote::PgQuoteRepository::new();
    // 在單一 transaction 內替換當日報價；失敗時資料庫自動 rollback，
    // 舊資料完整保留，錯誤則往上傳遞給呼叫端處理。
    let replaced_count = repo
        .replace_quotes_by_date(date, &domain_entities)
        .await
        .context("原子替換當日收盤報價失敗（資料庫已自動 rollback，舊資料未受影響）")?;

    // 資料庫確定寫入成功後，才併行更新記憶體與 TTL 快取。
    stream::iter(domain_entities)
        .for_each_concurrent(util::concurrent_limit_32(), |dq| async move {
            process_daily_quote(dq).await;
        })
        .await;
    tracing::info!("上市櫃收盤數據更新到資料庫完成: {}", replaced_count);

    Ok(replaced_count as usize)
}

async fn process_daily_quote(daily_quote: crate::domain::quote::entity::DailyQuote) {
    // 將最新報價更新至全域記憶體快取
    SHARE.set_stock_last_price(&daily_quote).await;

    // 取得記憶體快取之 key 值
    let daily_quote_memory_key = daily_quote.key_with_prefix();

    // 更新最後交易日的收盤價 TTL 快取（24 小時失效時間）
    TTL.daily_quote_set(
        daily_quote_memory_key,
        "".to_string(),
        Duration::from_secs(60 * 60 * 24),
    );
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    //use crossbeam::thread;
    use rayon::prelude::*;
    use tokio::time::sleep;

    use crate::infra::cache::SHARE;

    use super::*;

    //use std::time;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 execute");
        //let date = Local::now().date_naive();
        let date = NaiveDate::from_ymd_opt(2026, 4, 30).unwrap();
        // execute 內部已改為「先抓取、後於單一 transaction 原子替換」，
        // 不需要（也不應該）在呼叫前先手動刪除當日資料。

        match execute(date).await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to execute because {:?}", why);
            }
        }

        tracing::debug!("結束 execute");
        sleep(Duration::from_secs(1)).await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_thread() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 execute");

        let stock_repo = crate::infra::database::repository::stock::PgStockRepository::new();
        use crate::domain::registry::repository::StockRepository;
        let stocks = stock_repo.fetch_all_active().await.unwrap();
        let worker_count = num_cpus::get() * 100;
        //let dqs_arc = Arc::new(stocks);
        let counter = Arc::new(AtomicUsize::new(0));
        tracing::debug!("stocks:{}", stocks.len());
        /*  thread::scope(|scope| {
            for i in 0..worker_count {
                let dqs = Arc::clone(&dqs_arc);
                let counter = Arc::clone(&counter);

                scope.spawn(move |_| {
                    calculate_day_quotes_moving_average_worker(i, dqs.to_vec(), &counter);
                });
            }
        })
        .unwrap();*/
        stocks
            .par_iter()
            .with_min_len(worker_count)
            .for_each(|stock| {
                let index = counter.fetch_add(1, Ordering::SeqCst);
                calculate_day_quotes_moving_average_worker(index, stock);
            });

        tracing::debug!("結束 execute");
        sleep(Duration::from_secs(1)).await;
    }

    fn calculate_day_quotes_moving_average_worker(
        i: usize,
        dq: &crate::domain::registry::entity::Stock,
    ) {
        tracing::debug!("dq[{}]:{:?}", i, dq);
    }
}
