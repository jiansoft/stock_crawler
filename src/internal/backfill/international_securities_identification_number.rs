use crate::internal::{
    bot, cache::SHARE, crawler::twse, database::model, logging, util::datetime::Weekend,
    StockExchangeMarket,
};
use anyhow::*;
use chrono::Local;
use core::result::Result::Ok;
use std::fmt::Write;

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
    let result = match twse::international_securities_identification_number::visit(mode).await {
        None => {
            return Err(anyhow!(
                "Failed to visit because response is no data".to_string()
            ))
        }
        Some(result) => result,
    };

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
                    Some(&item)
                }
                None => Some(&item),
                _ => None,
            },
            Err(why) => {
                logging::error_file_async(format!("Failed to stocks.read because {:?}", why));
                continue;
            }
        };

        if let Some(isni) = new_stock {
            let stock = model::stock::Entity::from(isni.clone());
            if let Err(why) = stock.upsert().await {
                logging::error_file_async(format!("Failed to stock.upsert() because {:?}", why));
                continue;
            }

            let msg = format!("stock add or update {:?}", stock);
            if let Ok(mut stocks) = SHARE.stocks.write() {
                stocks.insert(stock.stock_symbol.to_string(), stock.clone());
            }
            let _ = writeln!(&mut to_bot_msg, "{}\r\n", msg);

            logging::info_file_async(msg);
        }
    }

    // todo 需要通知另一個服務已新增加一個股票代號
    if !to_bot_msg.is_empty() {
        if let Err(why) = bot::telegram::send_to_allowed(&to_bot_msg).await {
            logging::error_file_async(format!("Failed to send_to_allowed because {:?}", why));
        }
    }

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
