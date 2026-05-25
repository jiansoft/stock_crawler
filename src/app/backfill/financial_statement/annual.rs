use crate::{
    app::backfill::financial_statement::update_roe_and_roa_for_zero_values,
    core::logging,
    core::util::{self, datetime::Weekend},
    infra::crawler::wespai,
    infra::database::table::{financial_statement, stock},
};
use anyhow::Result;
use chrono::Local;
use scopeguard::defer;

/// 更新台股年報
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }

    logging::info_file_async("更新台股年度財報開始");
    defer! {
       logging::info_file_async("更新台股年度財報結束");
    }

    let cache_key = "financial_statement:annual";
    let is_jump = crate::infra::nosql::redis::CLIENT
        .get_bool(cache_key)
        .await?;
    if is_jump {
        return Ok(());
    }

    let profits = wespai::profit::visit().await?;
    if profits.is_empty() {
        logging::warn_file_async("profits from wespai is empty".to_string());
        return Ok(());
    }

    let annual = financial_statement::fetch_annual(profits[0].year).await?;
    let exist_fs = util::map::vec_to_hashmap(annual);
    // 將過濾後符合條件的財報資料轉換成實體 Vector，以便進行批量寫入
    let statements: Vec<_> = profits
        .into_iter()
        .filter(|profit| !stock::is_preference_shares(&profit.security_code))
        .filter(|profit| {
            let key = format!(
                "{}-{}-{}",
                profit.security_code, profit.year, profit.quarter
            );
            !exist_fs.contains_key(&key)
        })
        .map(financial_statement::FinancialStatement::from)
        .collect();

    // 如果有需要新增或更新的財報，則呼叫 batch_upsert 批次寫入資料庫
    if !statements.is_empty() {
        if let Err(why) = financial_statement::FinancialStatement::batch_upsert(&statements).await {
            logging::error_file_async(format!(
                "Failed to FinancialStatement.batch_upsert because {:?}",
                why
            ));
        }
    }

    update_roe_and_roa_for_zero_values(None).await?;

    crate::infra::nosql::redis::CLIENT
        .set(cache_key, true, 60 * 60 * 24 * 7)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{core::logging, infra::cache::SHARE};

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
