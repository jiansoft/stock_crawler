use anyhow::Result;
use chrono::Local;
use futures::future;

use crate::{
    internal::{
        crawler::wespai,
        database::table::{financial_statement, stock},
        nosql,
    },
    logging,
    util::datetime::Weekend,
};

/// 更新台股年報
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }

    let cache_key = "financial_statement:annual";
    let is_jump = nosql::redis::CLIENT.get_bool(cache_key).await?;
    if is_jump {
        return Ok(());
    }

    let profits = wespai::profit::visit().await?;
    if profits.is_empty() {
        logging::warn_file_async("profits from wespai is empty".to_string());
        return Ok(());
    }

    let annual = financial_statement::fetch_annual(profits[0].year).await?;
    let exist_fs = financial_statement::vec_to_hashmap(annual);
    let upsert_futures: Vec<_> = profits
        .into_iter()
        .filter(|profit| !stock::is_preference_shares(&profit.security_code))
        .filter(|profit| !exist_fs.contains_key(&profit.security_code))
        .map(|profit| {
            let fs = financial_statement::FinancialStatement::from(profit);
            fs.upsert()
        })
        .collect();
    let results = future::join_all(upsert_futures).await;
    for result in results {
        if let Err(why) = result {
            logging::error_file_async(format!(
                "Failed to FinancialStatement.upsert because {:?}",
                why
            ));
        }
    }

    nosql::redis::CLIENT
        .set(cache_key, true, 60 * 60 * 24 * 7)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{
        internal::cache::SHARE,
        logging
    };

    use super::*;

    #[tokio::test]
    #[ignore]
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
