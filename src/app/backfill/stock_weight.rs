use std::sync::Arc;

use anyhow::{Context, Result};
use futures::{StreamExt, stream};
use scopeguard::defer;
use tokio::sync::Mutex;

use crate::{
    app::backfill::acl::{SaveStockWeightCommand, StockWeightAclMapper},
    core::declare::StockExchange,
    core::util,
    domain::registry::repository::StockRepository,
    infra::crawler::taifex,
    infra::database::repository::stock::PgStockRepository,
};

/// <summary>
/// 執行個股權值比重回填任務。
/// 從期交所 (Taifex) 爬取最新的上市與上櫃個股權值比重資料，將所有權重重置後，再批次更新。
/// </summary>
pub async fn execute() -> Result<()> {
    tracing::info!("更新個股權值比重開始");
    defer! {
       tracing::info!("更新個股權值比重結束");
    }
    // 建立 Thread-safe 容器，用以收集並行處理的權重資料
    let stock_weights = Arc::new(Mutex::new(Vec::with_capacity(2000)));
    let exchanges = vec![StockExchange::TPEx, StockExchange::TWSE];

    // 針對不同的交易所 (上市與上櫃) 啟動並行任務進行下載與轉譯
    let tasks: Vec<_> = exchanges
        .into_iter()
        .map(|exchange| handle_stock_exchange(exchange, Arc::clone(&stock_weights)))
        .collect();

    // 等待所有下載與轉譯任務完成
    futures::future::try_join_all(tasks)
        .await
        .context("Failed to handle stock weight tasks")?;

    // 取得鎖定以讀取收集好的個股權重
    let weights = stock_weights.lock().await;
    save_weights(&PgStockRepository::new(), &weights).await
}

/// 先把所有個股權重歸零（避免舊權重殘留），再並行寫入 `weights`；`weights` 為空時不動資料庫。
///
/// # Errors
///
/// 歸零失敗時回傳錯誤；單檔寫入失敗只記錄。
async fn save_weights(
    repo: &impl StockRepository,
    weights: &[SaveStockWeightCommand],
) -> Result<()> {
    if weights.is_empty() {
        return Ok(());
    }
    repo.zeroed_out_weights()
        .await
        .context("Failed to zero out stock weights in registry repository")?;

    // 使用並行串流，限制最大並行度更新每檔個股的權值
    stream::iter(weights)
        .for_each_concurrent(util::concurrent_limit_16(), |sw| async move {
            if let Err(why) = repo.update_weight(&sw.symbol, sw.weight).await {
                tracing::error!("Failed to update stock weight: {:#?}", why);
            }
        })
        .await;

    Ok(())
}

/// <summary>
/// 處理指定證券交易所的個股權值比重爬取與轉譯。
/// </summary>
/// <param name="exchange">證券交易所類型 (如上市或上櫃)</param>
/// <param name="stock_weights">共享的權重命令收集容器</param>
async fn handle_stock_exchange(
    exchange: StockExchange,
    stock_weights: Arc<Mutex<Vec<SaveStockWeightCommand>>>,
) -> Result<()> {
    // 爬取期交所個股權重資料 DTO
    let res = taifex::stock_weight::visit(exchange)
        .await
        .with_context(|| format!("Failed to visit taifex for exchange {:?}", exchange))?;

    // 權值頁一定有成分股；抓到 0 檔代表來源異常（維護頁、改版、錯誤頁）。
    // 此時回錯讓整個任務中止，否則 `save_weights` 會先把全部權重歸零、只寫回另一個交易所（H38）。
    if res.is_empty() {
        anyhow::bail!("期交所 {exchange:?} 權值頁沒有任何資料，不更新權值以免清空");
    }

    // 將 DTO 轉譯為應用層的儲存權重命令 (SaveStockWeightCommand)
    let new_weights: Vec<SaveStockWeightCommand> = res
        .into_iter()
        .map(|dto| StockWeightAclMapper::from_dto(&dto))
        .collect();

    // 鎖定容器並將結果寫入
    let mut weights = stock_weights.lock().await;
    weights.extend(new_weights);

    Ok(())
}

#[cfg(test)]
mod tests;
