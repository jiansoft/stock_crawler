use crate::{
    app::backfill::acl::DelistedCompanyAclMapper, core::util::datetime::Weekend,
    domain::registry::repository::StockRepository, infra::crawler::twse,
    infra::database::repository::stock::PgStockRepository,
};
use anyhow::Result;
use chrono::Local;
use scopeguard::defer;

/// 更新資料庫中終止上市的公司
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }
    tracing::info!("更新下市的股票開始");
    defer! {
       tracing::info!("更新下市的股票結束");
    }
    let delisted = twse::suspend_listing::visit().await?;
    let repo = PgStockRepository::new();

    for company in delisted {
        // 透過防腐層轉譯為內部命令，內含格式與民國年分過濾邏輯
        if let Some(cmd) = DelistedCompanyAclMapper::from_suspend_listing(&company)
            && let Some(stock) = repo.find_by_symbol(&cmd.symbol).await?
        {
            if stock.suspend_listing() {
                continue;
            }

            let mut another = stock.clone();
            another.update_suspension(true);

            if let Err(why) = repo.save(&another).await {
                tracing::error!("Failed to update_suspend_listing because {:?}", why);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::infra::cache::SHARE;

    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 execute");

        match execute().await {
            Ok(_) => {
                tracing::debug!("execute executed successfully.");
            }
            Err(why) => {
                tracing::debug!("Failed to execute because {:?}", why);
            }
        }

        tracing::debug!("結束 execute");
    }
}
