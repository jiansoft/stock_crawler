use crate::{
    internal::cache_share::CACHE_SHARE,
    internal::crawler::twse,
    internal::database::model,
    internal::util::datetime::Weekend,
    internal::{bot, StockExchangeMarket},
    logging,
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
    for insi in result {
        let new_stock = match CACHE_SHARE.stocks.read() {
            Ok(stocks_cache) => match stocks_cache.get(&insi.stock_symbol) {
                Some(stock_db)
                    if stock_db.stock_industry_id != insi.industry_id
                        || stock_db.stock_exchange_market_id
                            != insi.exchange_market.stock_exchange_market_id
                        || stock_db.name != insi.name =>
                {
                    Some(&insi)
                }
                None => Some(&insi),
                _ => None,
            },
            Err(why) => {
                logging::error_file_async(format!("Failed to stocks.read because {:?}", why));
                continue;
            }
        };

        if let Some(isni) = new_stock {
            let stock = model::stock::Entity::from(isni.clone());
            stock.upsert().await?;
            let msg = format!("stock add or update {:?}", stock);
            if let Ok(mut stocks) = CACHE_SHARE.stocks.write() {
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
    use crate::logging;

    #[tokio::test]
    async fn test_execute() {
        dotenv::dotenv().ok();
        CACHE_SHARE.load().await;
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
