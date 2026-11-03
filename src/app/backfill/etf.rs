use crate::{
    app::backfill,
    app::backfill::acl::EtfAclMapper,
    core::util::datetime::Weekend,
    infra::cache::SHARE,
    infra::crawler::{share::EtfInfo, tpex, twse},
};

use std::future::Future;

use anyhow::Result;
use chrono::Local;
use scopeguard::defer;

/// 執行台股 ETF 資訊的同步與更新。
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }
    sync(twse::etf::visit(), tpex::etf::visit()).await
}

/// 依序以上市、上櫃 ETF 清單更新證券主檔；任一市場抓取失敗只記錄，另一市場照常更新。
///
/// # Errors
///
/// 只有更新主檔的流程本身回傳錯誤時才回傳（單檔寫入失敗只記錄）。
async fn sync(
    listed: impl Future<Output = Result<Vec<EtfInfo>>>,
    otc: impl Future<Output = Result<Vec<EtfInfo>>>,
) -> Result<()> {
    tracing::info!("更新台股 ETF 資訊開始");
    defer! {
       tracing::info!("更新台股 ETF 資訊結束");
    }

    // 1. 抓取上市 ETF 資料
    match listed.await {
        Ok(items) => update_stocks(items).await?,
        Err(why) => tracing::error!("處理上市 ETF 市場失敗: {:?}", why),
    }

    // 2. 抓取上櫃 ETF 資料
    match otc.await {
        Ok(items) => update_stocks(items).await?,
        Err(why) => tracing::error!("處理上櫃 ETF 市場失敗: {:?}", why),
    }

    Ok(())
}

/// 批次更新股票資訊到資料庫。
async fn update_stocks(items: Vec<EtfInfo>) -> Result<()> {
    for item in items {
        let Some(market) = SHARE.get_exchange_market(item.market.serial()) else {
            tracing::error!(
                symbol = %item.stock_symbol,
                market = item.market.serial(),
                "略過 ETF：交易市場對照資料不存在"
            );
            continue;
        };
        let market_id = market.stock_exchange_market_id;

        let is_new_or_changed = backfill::is_stock_identity_new_or_changed(
            &item.stock_symbol,
            item.industry_id,
            market_id,
            &item.name,
        )
        .await;

        if is_new_or_changed {
            let cmd = EtfAclMapper::from_etf(&item);
            if let Err(why) = update_stock_info(&cmd).await {
                tracing::error!("更新 ETF {} 資訊失敗: {:?}", item.stock_symbol, why);
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
mod tests;
