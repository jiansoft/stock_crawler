use crate::internal::{
    bot, cache::SHARE, crawler::twse, database::model, logging, util::datetime::Weekend,
    StockExchangeMarket,
};
use anyhow::*;
use chrono::Local;
use std::{fmt::Write, result::Result::Ok};

/// 更新資料庫新上市股票的或更新其交易所的市場編號、股票的產業分類、名稱等欄位
pub async fn execute() -> Result<()> {
    if Local::now().is_weekend() {
        return Ok(());
    }

    for market in StockExchangeMarket::iterator() {
        if let Err(why) = process_market(market).await {
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
            if let Err(e) = update_stock_info(&item, &mut to_bot_msg).await {
                logging::error_file_async(format!(
                    "Failed to update stock info for {} because {:?}",
                    item.stock_symbol, e
                ));
            }
        }
    }

    if !to_bot_msg.is_empty() {
        if let Err(why) = bot::telegram::send(&to_bot_msg).await {
            logging::error_file_async(format!("Failed to send_to_allowed because {:?}", why));
        }
    }

    Ok(())
}

async fn update_stock_info(
    stock: &twse::international_securities_identification_number::Entity,
    msg: &mut String,
) -> Result<()> {
    let stock = model::stock::Entity::from(stock.clone());
    stock.upsert().await.map_err(|e| {
        logging::error_file_async(format!("Failed to stock.upsert() because {:?}", e));
        anyhow!(e)
    })?;

    if let Ok(mut stocks) = SHARE.stocks.write() {
        stocks.insert(stock.stock_symbol.to_string(), stock.clone());
    }

    let log_msg = format!("stock add or update {:?}", stock);
    writeln!(msg, "{}\r\n", log_msg).ok(); // We don't care if this write fails, so use `.ok()`.
    logging::info_file_async(log_msg);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::logging;

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
