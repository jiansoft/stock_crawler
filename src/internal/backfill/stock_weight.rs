use core::result::Result::Ok;

use anyhow::*;
use futures::{stream, StreamExt};

use crate::internal::{crawler::taifex, database::table::stock, logging};

/// 查詢 taifex 個股權值比重
pub async fn execute() -> Result<()> {
    let taifex_weights = taifex::stock_weight::visit().await?;
    let stock_weights = stock::extension::weight::from(taifex_weights);
    stream::iter(stock_weights)
        .for_each_concurrent(None, |stock_weight| async move {
            if let Err(why) = stock_weight.update().await {
                logging::error_file_async(format!(
                    "Failed to stock_weight.update because {:?}",
                    why
                ));
            }
        })
        .await;

    Ok(())
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
