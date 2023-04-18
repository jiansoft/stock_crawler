use crate::internal::util::datetime;
use crate::{internal::crawler::yahoo, internal::database::model, logging};
use anyhow::*;
use chrono::{Datelike, Duration, Local};
use core::result::Result::Ok;

/// 將未有上季度財報的股票，到雅虎財經下載後回寫到 financial_statement 表
pub async fn execute() -> Result<()> {
    let previous_quarter = Local::now() - Duration::days(120);
    let year = previous_quarter.year();
    let quarter = datetime::month_to_quarter(previous_quarter.month());
    let stocks = model::stock::fetch_stocks_without_financial_statement(year, quarter).await?;
    for stock in stocks {
        if stock.is_preference_shares() {
            continue;
        }

        match yahoo::profile::visit(&stock.stock_symbol).await {
            Ok(stock_profile) => {
                if year != stock_profile.year || quarter != stock_profile.quarter {
                    logging::warn_file_async(format!(
                        "the year or quarter retrieved from Yahoo is inconsistent with the current one. current year:{} ,quarter:{}\r\n{:#?}",
                        year, quarter, stock_profile
                    ));
                    continue;
                }

                let fs = model::financial_statement::Entity::from(stock_profile);

                match fs.upsert().await {
                    Ok(_) => {
                        logging::info_file_async(format!(
                            "financial_statement upsert executed successfully. \r\n{:#?}",
                            stock
                        ));
                    }
                    Err(why) => {
                        logging::error_file_async(format!("Failed to upsert because {:?}", why));
                    }
                }
            }
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to yahoo::profile::visit because {:?}",
                    why
                ));
            }
        };
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::cache::SHARE;
    use crate::logging;

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
