use std::fmt::Write;

use anyhow::{anyhow, Result};
use chrono::Local;
use rust_decimal::prelude::ToPrimitive;

use crate::{
    internal::{
        bot, cache::SHARE, crawler::twse, database::table, rpc, rpc::stock, StockExchangeMarket,
    },
    logging,
    util::datetime::Weekend,
};

/// 更新資料庫新上市股票的或更新其交易所的市場編號、股票的產業分類、名稱等欄位
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }

    let tasks: Vec<_> = StockExchangeMarket::iterator()
        .map(process_market)
        .collect();

    let results = futures::future::join_all(tasks).await;
    for result in results {
        if let Err(why) = result {
            logging::error_file_async(format!("Failed to process_market because {:?}", why));
        }
    }

    Ok(())
}

async fn process_market(mode: StockExchangeMarket) -> Result<()> {
    let result = twse::international_securities_identification_number::visit(mode).await?;
    let mut to_bot_msg = String::with_capacity(1024);
    for item in result {
        let new_stock = match SHARE.stocks.read() {
            Ok(stocks_cache) => match stocks_cache.get(&item.stock_symbol) {
                Some(stock_db)
                    if stock_db.stock_industry_id != item.industry_id
                        || stock_db.stock_exchange_market_id
                            != item.exchange_market.stock_exchange_market_id
                        || stock_db.name != item.name =>
                {
                    true
                }
                None => true,
                _ => false,
            },
            Err(why) => {
                logging::error_file_async(format!("Failed to stocks.read because {:?}", why));
                continue;
            }
        };

        if new_stock {
            if let Err(why) = update_stock_info(&item, &mut to_bot_msg).await {
                logging::error_file_async(format!(
                    "Failed to update stock info for {} because {:?}",
                    item.stock_symbol, why
                ));
            }
        }
    }

    if !to_bot_msg.is_empty() {
        if let Err(why) = bot::telegram::send(&to_bot_msg).await {
            logging::error_file_async(format!("Failed to send because {:?}", why));
        }
    }

    Ok(())
}

async fn update_stock_info(
    stock: &twse::international_securities_identification_number::InternationalSecuritiesIdentificationNumber,
    msg: &mut String,
) -> Result<()> {
    let stock = table::stock::Stock::from(stock.clone());
    stock
        .upsert()
        .await
        .map_err(|why| anyhow!("Failed to stock.upsert() because {:?}", why))?;

    if let Ok(mut stocks) = SHARE.stocks.write() {
        stocks.insert(stock.stock_symbol.to_string(), stock.clone());
    }

    let log_msg = format!("stock add or update {:?}", stock);
    writeln!(msg, "{}\r\n", log_msg).ok(); // We don't care if this write fails, so use `.ok()`.
    logging::info_file_async(log_msg);

    //通知 go service
    let request = stock::StockInfoRequest {
        stock_symbol: stock.stock_symbol.to_string(),
        name: stock.name.to_string(),
        stock_exchange_market_id: stock.stock_exchange_market_id,
        stock_industry_id: stock.stock_industry_id,
        net_asset_value_per_share: stock.net_asset_value_per_share.to_f64().unwrap_or(0.0),
        suspend_listing: false,
    };

    if let Err(why) = rpc::client::stock_service::push_stock_info_to_go_service(request).await {
        logging::error_file_async(format!(
            "Failed to push_stock_info_to_go_service for {} because {:?}",
            stock.stock_symbol, why
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 execute".to_string());

        match execute().await {
            Ok(_) => {
                logging::debug_file_async("完成 execute".to_string());
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because {:?}", why));
            }
        }

        logging::debug_file_async("結束 execute".to_string());
    }
}
