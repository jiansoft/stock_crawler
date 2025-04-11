use std::sync::Arc;

use anyhow::{Context, Result};
use futures::{stream, StreamExt};
use scopeguard::defer;
use tokio::sync::Mutex;

use crate::{
    crawler::taifex,
    database::table::stock::{self, extension::weight::SymbolAndWeight},
    declare::StockExchange,
    logging, util,
};

/// 查詢 taifex 個股權值比重
pub async fn execute() -> Result<()> {
    logging::info_file_async("更新個股權值比重開始");
    defer! {
       logging::info_file_async("更新個股權值比重結束");
    }
    let stock_weights = Arc::new(Mutex::new(Vec::with_capacity(2000)));
    let exchanges = vec![StockExchange::TPEx, StockExchange::TWSE];
    // Process each exchange concurrently
    let tasks: Vec<_> = exchanges.into_iter()
        .map(|exchange| handle_stock_exchange(exchange, Arc::clone(&stock_weights)))
        .collect();

    // Await all tasks
    futures::future::try_join_all(tasks).await.context("Failed to handle stock weight tasks")?;

    // Acquire the lock to update weights
    let weights = stock_weights.lock().await;

    if !weights.is_empty() {
        SymbolAndWeight::zeroed_out().await.context("Failed to zero out SymbolAndWeight")?;
        stream::iter(weights.clone())
            .for_each_concurrent(util::concurrent_limit_16(), |sw| async move {
                if let Err(why) = sw.update().await {
                    logging::error_file_async(format!(
                        "Failed to update stock weight: {:#?}",
                        why
                    ));
                }
            })
            .await;
    }

    Ok(())
}
/// Handle the processing of stock weights for a given exchange
async fn handle_stock_exchange(
    exchange: StockExchange,
    stock_weights: Arc<Mutex<Vec<SymbolAndWeight>>>,
) -> Result<()> {
    let res = taifex::stock_weight::visit(exchange)
        .await
        .with_context(|| format!("Failed to visit taifex for exchange {:?}", exchange))?;
    let new_weights = stock::extension::weight::from(res);
    let mut weights = stock_weights.lock().await;

    weights.extend(new_weights);

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{cache::SHARE, logging};

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute".to_string());

        match execute().await {
            Ok(_) => {
                logging::debug_file_async("成功執行 execute".to_string());
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        logging::debug_file_async("結束 execute".to_string());
    }
}
