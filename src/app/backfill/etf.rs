use crate::{
    app::backfill,
    app::backfill::acl::EtfAclMapper,
    core::logging,
    core::util::datetime::Weekend,
    infra::crawler::{share::EtfInfo, tpex, twse},
};

use anyhow::Result;
use chrono::Local;
use scopeguard::defer;

/// 執行台股 ETF 資訊的同步與更新。
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }

    logging::info_file_async("更新台股 ETF 資訊開始");
    defer! {
       logging::info_file_async("更新台股 ETF 資訊結束");
    }

    // 1. 抓取上市 ETF 資料
    match twse::etf::visit().await {
        Ok(items) => update_stocks(items).await?,
        Err(why) => logging::error_file_async(format!("處理上市 ETF 市場失敗: {:?}", why)),
    }

    // 2. 抓取上櫃 ETF 資料
    match tpex::etf::visit().await {
        Ok(items) => update_stocks(items).await?,
        Err(why) => logging::error_file_async(format!("處理上櫃 ETF 市場失敗: {:?}", why)),
    }

    Ok(())
}

/// 批次更新股票資訊到資料庫。
async fn update_stocks(items: Vec<EtfInfo>) -> Result<()> {
    for item in items {
        let is_new_or_changed = backfill::is_stock_identity_new_or_changed(
            &item.stock_symbol,
            item.industry_id,
            item.exchange_market.stock_exchange_market_id,
            &item.name,
        )
        .await;

        if is_new_or_changed {
            let cmd = EtfAclMapper::to_registration_command(&item);
            if let Err(why) = update_stock_info(&cmd).await {
                logging::error_file_async(format!(
                    "更新 ETF {} 資訊失敗: {:?}",
                    item.stock_symbol, why
                ));
            }
        }
    }

    Ok(())
}

async fn update_stock_info(cmd: &backfill::acl::RegisterStockCommand) -> Result<()> {
    use crate::app::event::handlers::get_global_dispatcher;
    use crate::domain::registry::entity::Stock;
    use crate::domain::registry::repository::StockRepository;
    use crate::infra::database::repository::stock::PgStockRepository;

    let repo = PgStockRepository::new();

    // 1. 獲取已存在的證券主檔或註冊一個新的，並使用業務方法更新識別資訊
    let mut stock = match repo.find_by_symbol(&cmd.symbol).await? {
        Some(mut existing) => {
            existing.change_identity(cmd.name.clone(), cmd.market_id, cmd.industry_id);
            existing
        }
        None => Stock::register(
            cmd.symbol.clone(),
            cmd.name.clone(),
            cmd.market_id,
            cmd.industry_id,
        ),
    };

    // 2. 寫入資料庫：利用 Repository 保存，會自動觸發快取同步
    repo.save(&stock)
        .await
        .map_err(|why| anyhow::anyhow!("資料庫 upsert 失敗: {:?}", why))?;

    // 3. 提取領域事件並非同步派發
    let events = stock.pull_events();
    get_global_dispatcher().dispatch_async(events).await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::logging;
    use crate::infra::cache::SHARE;

    #[tokio::test]
    #[ignore]
    async fn test_execute_etf() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute_etf".to_string());

        match execute().await {
            Ok(_) => {
                logging::debug_file_async("完成 execute_etf".to_string());
            }
            Err(why) => {
                logging::debug_file_async(format!("執行失敗: {:?}", why));
            }
        }
    }
}
