use std::fmt::Write;

use anyhow::Result;
use chrono::Local;
use rust_decimal::{
    Decimal,
    prelude::ToPrimitive,
};
use rust_decimal_macros::dec;

use crate::{
    bot,
    cache::SHARE,
    crawler,
    declare,
    nosql,
    util::{
        map::Keyable,
        convert::FromValue
    }
};

pub async fn execute() -> Result<()> {
    let ps = crawler::twse::public::visit().await?;
    let mut msg = String::with_capacity(2048);
    let now = Local::now().date_naive();
    
    for stock in ps {
        if stock.market == "中央登錄公債" {
            continue;
        }
        
        if let (Some(start), Some(end), Some(offering_price)) = (
            stock.offering_start_date,
            stock.offering_end_date,
            stock.offering_price,
        ) {
            if now >= start && now <= end {
                let cache_key = stock.key_with_prefix();
                let is_jump = nosql::redis::CLIENT.get_bool(&cache_key).await?;

                if is_jump {
                    continue;
                }
                
                let stock_last_price = SHARE.get_stock_last_price(&stock.stock_symbol).await;
                let last_price = match stock_last_price {
                    None => String::from(" - "),
                    Some(last_quote) => match last_quote.closing_price.to_f64() {
                        None => String::from(" - "),
                        Some(price) => price.to_string(),
                    },
                };
                
                let last_price_dec = last_price.get_decimal(None);
                let price_change = calculate_price_change(offering_price,last_price_dec);
                let _ = writeln!(
                    &mut msg, "{stock_symbol} {stock_name} 起迄日︰{start}~{end} 承銷價︰{offering_price} 參考價︰{last_price} {price_change}發行市場:{market}",
                    market = stock.market,
                    stock_symbol = stock.stock_symbol,
                    stock_name = stock.stock_name,
                    start = start,
                    end = end,
                    offering_price = offering_price,
                    last_price = last_price,
                    price_change = price_change
                );

                let mut duration = (end - now).num_seconds() as usize;

                if duration == 0 {
                    duration = declare::ONE_DAYS_IN_SECONDS;
                }

                nosql::redis::CLIENT.set(cache_key, true, duration).await?;
            }
        }
    }

    if !msg.is_empty() {
        let to_bot_msg = format!("{} 可以申購的股票如下︰\n{}", now, msg);
        let _ = bot::telegram::send(&to_bot_msg).await;
        return Ok(());
    }

    Ok(())
}


fn calculate_price_change(offering_price: Decimal, last_price: Decimal) -> String {
    if offering_price == Decimal::ZERO || last_price == Decimal::ZERO {
        return String::from(" - ");
    }
    //價差 = 參考價 - 承銷價
    //價差百分比 = 價差 / 參考價 * 100%
    let gap = last_price - offering_price;
    let change = (gap / last_price) * dec!(100);
    format!("價差︰{gap} ({change}%) ", gap = gap, change = change.round_dp(2))
}

#[cfg(test)]
mod tests {
    use crate::logging;

    use super::*;

    #[tokio::test]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::info_file_async("開始 execute".to_string());
        //let date = NaiveDate::from_ymd_opt(2023, 6, 15);
        //let today: NaiveDate = Local::today().naive_local();
        match execute().await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to execute because: {:?}", why));
            }
        }

        logging::info_file_async("結束 execute".to_string());
    }
}
