use std::sync::Arc;

use anyhow::Result;
use futures::{stream, StreamExt};
use tokio::sync::Mutex;

use crate::{
    crawler::taifex,
    database::table::stock::{self, extension::weight::SymbolAndWeight},
    declare::StockExchange,
    logging, util,
};

/// 查詢 taifex 個股權值比重
pub async fn execute() -> Result<()> {
    let stock_weights = Arc::new(Mutex::new(Vec::with_capacity(2000)));
    let error_occurred = Arc::new(Mutex::new(false));
    let tpex_task = handle_stock_exchange(
        StockExchange::TPEx,
        stock_weights.clone(),
        error_occurred.clone(),
    );
    let twse_task = handle_stock_exchange(
        StockExchange::TWSE,
        stock_weights.clone(),
        error_occurred.clone(),
    );
    // Handle the results of the joined tasks
    let (tpex_result, twse_result) = tokio::join!(tpex_task, twse_task);

    // Check and handle errors from each task
    tpex_result?;
    twse_result?;

    let weights = stock_weights.lock().await;
    let error = error_occurred.lock().await;

    if !*error && !weights.is_empty() {
        SymbolAndWeight::zeroed_out().await?;
        stream::iter(weights.clone())
            .for_each_concurrent(util::concurrent_limit_16(), |sw| async move {
                if let Err(why) = sw.update().await {
                    logging::error_file_async(format!(
                        "Failed to stock_weight.update because {:#?}",
                        why
                    ));
                }
            })
            .await;
    }

    Ok(())
}

async fn handle_stock_exchange(
    exchange: StockExchange,
    stock_weights: Arc<Mutex<Vec<SymbolAndWeight>>>,
    error_occurred: Arc<Mutex<bool>>,
) -> Result<()> {
    let result = fetch_stock_weights(exchange).await;
    let mut weights = stock_weights.lock().await;

    match result {
        Ok(new_weights) => {
            weights.extend(new_weights);
        }
        Err(why) => {
            let mut error = error_occurred.lock().await;
            *error = true;
            logging::error_file_async(format!(
                "Failed to fetch_stock_weights {:?} because {:#?}",
                exchange, why
            ));
        }
    }
    Ok(())
}

async fn fetch_stock_weights(stock_exchange: StockExchange) -> Result<Vec<SymbolAndWeight>> {
    let res = taifex::stock_weight::visit(stock_exchange).await?;
    let weights = stock::extension::weight::from(res);

    Ok(weights)
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
