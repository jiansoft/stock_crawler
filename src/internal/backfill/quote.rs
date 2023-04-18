use crate::{
    internal::crawler::twse,
    logging,
    internal::database::model::daily_quote,
    internal::crawler::tpex
};
use anyhow::*;
use chrono::Local;
use core::result::Result::Ok;

/// 調用  twse API 取得台股月營收
pub async fn execute() -> Result<()> {
    let now = Local::now();
    let mut results: Vec<daily_quote::Entity> = Vec::with_capacity(2048);

    if let Some(twse) = twse::quote::visit(now).await {
        results.extend(twse);
    }

    //上櫃
    if let Some(twse) = tpex::quote::visit(now).await {
        results.extend(twse);
    }

    logging::debug_file_async(format!(
        "result({})",
        results.len()
    ));

    for result in results {
        match result.upsert().await {
            Ok(_) => {
                logging::debug_file_async(format!(
                    "result:{:#?}",
                   result
                ));
            }
            Err(why) => {
                logging::error_file_async(format!("Failed to quote.upsert because {:?}", why));
            }
        }
    }

    Ok(())
}



#[cfg(test)]
mod tests {
    use crate::internal::cache::SHARE;
    use super::*;
    use crate::logging;

    #[tokio::test]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute".to_string());

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        logging::debug_file_async("結束 execute".to_string());
    }
}
