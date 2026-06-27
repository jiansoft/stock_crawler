use crate::{
    app::backfill::acl::FinancialStatementAclMapper,
    app::backfill::financial_statement::update_roe_and_roa_for_zero_values,
    core::util::datetime::Weekend,
    domain::financial::entity::FinancialStatement as DomainFinancialStatement,
    domain::financial::repository::FinancialRepository, domain::registry::entity::StockSymbol,
    infra::crawler::wespai, infra::database::repository::financial::PgFinancialRepository,
};
use anyhow::Result;
use chrono::Local;
use scopeguard::defer;

/// <summary>
/// 更新台股年度財務報告。
/// 下載年度財報資料，過濾已存在與特別股資料後，寫入領域倉儲。
/// </summary>
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }

    tracing::info!("更新台股年度財報開始");
    defer! {
       tracing::info!("更新台股年度財報結束");
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
        tracing::warn!("profits from wespai is empty");
        return Ok(());
    }

    let financial_repo = PgFinancialRepository::new();

    // 依據年份讀取現有的年度財報
    let annual = financial_repo
        .fetch_annual_statements(profits[0].year)
        .await?;
    // 將現有的年度財報建立為以 "股票代碼-年度-季度" 為鍵值的 HashMap，供過濾使用
    let exist_fs: std::collections::HashMap<String, DomainFinancialStatement> = annual
        .into_iter()
        .map(|fs| {
            let key = format!("{}-{}-{}", fs.security_code, fs.year, fs.quarter);
            (key, fs)
        })
        .collect();

    // 將過濾後符合條件的財報資料轉換成領域實體 Vector
    let statements: Vec<DomainFinancialStatement> = profits
        .into_iter()
        // 過濾掉特別股/優先股 (特別股代碼中會包含英文字母)
        .filter(|profit| !StockSymbol(profit.security_code.clone()).is_preference())
        // 過濾掉已經存在於資料庫中的財報
        .filter(|profit| {
            let key = format!(
                "{}-{}-{}",
                profit.security_code, profit.year, profit.quarter
            );
            !exist_fs.contains_key(&key)
        })
        // 透過防腐層轉譯器將 DTO 轉成領域實體
        .map(FinancialStatementAclMapper::from_wespai)
        .collect();

    // 如果有需要新增或更新的財報，則呼叫領域倉儲的批次寫入
    if !statements.is_empty()
        && let Err(why) = financial_repo
            .batch_save_financial_statements(&statements)
            .await
    {
        tracing::error!(
            "Failed to financial_repo.batch_save_financial_statements because {:?}",
            why
        );
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
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 execute");

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to execute because {:?}", why);
            }
        }

        tracing::debug!("結束 execute");
    }
}
