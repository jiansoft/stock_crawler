use core::result::Result::Ok;
use std::cmp::max;

use anyhow::*;
use futures::{stream, StreamExt};

use crate::internal::{
    crawler::taifex,
    database::table::{stock, stock::extension::weight::SymbolAndWeight},
    logging, StockExchange,
};

/// 查詢 taifex 個股權值比重
pub async fn execute() -> Result<()> {
    let tpex = fetch_stock_weights(StockExchange::TPEx);
    let twse = fetch_stock_weights(StockExchange::TWSE);
    let (res_tpex, res_twse) = tokio::join!(tpex, twse);
    let mut stock_weights: Vec<SymbolAndWeight> = Vec::with_capacity(2000);

    if let Ok(tpex_weights) = res_tpex {
        stock_weights.extend(tpex_weights);
    }
    if let Ok(twse_weights) = res_twse {
        stock_weights.extend(twse_weights);
    }

    stream::iter(stock_weights)
        .for_each_concurrent(Some(max(8, num_cpus::get() * 4)), |sw| async move {
            if let Err(why) = sw.update().await {
                logging::error_file_async(format!(
                    "Failed to stock_weight.update because {:?}",
                    why
                ));
            }
        })
        .await;

    Ok(())
}

async fn fetch_stock_weights(stock_exchange: StockExchange) -> Result<Vec<SymbolAndWeight>> {
    let res = taifex::stock_weight::visit(stock_exchange).await?;
    let weights = stock::extension::weight::from(res);
    Ok(weights)
}

#[cfg(test)]
mod tests {
    use crate::internal::cache::SHARE;
    use crate::internal::logging;

    use super::*;

    #[tokio::test]
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
