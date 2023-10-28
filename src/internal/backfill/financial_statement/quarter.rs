use core::result::Result::Ok;

use anyhow::*;
use chrono::{Datelike, Duration, Local};

use crate::{
    internal::{
        calculation, crawler::yahoo, database::table, logging, nosql,
    },
    util::datetime
};

/// 將未有上季度財報的股票，到雅虎財經下載後回寫到 financial_statement 表
pub async fn execute() -> Result<()> {
    let cache_key = "financial_statement:quarter";
    let is_jump = nosql::redis::CLIENT.get_bool(cache_key).await?;
    if is_jump {
        return Ok(());
    }

    let now = Local::now();
    let previous_quarter = now - Duration::days(130);
    let year = previous_quarter.year();
    let quarter = datetime::month_to_quarter(previous_quarter.month());
    let stocks = table::stock::fetch_stocks_without_financial_statement(year, quarter).await?;
    let mut success_update_count = 0;
    for stock in stocks {
        if stock.is_preference_shares() {
            continue;
        }

        let profile = match yahoo::profile::visit(&stock.stock_symbol).await {
            Ok(profile) => profile,
            Err(why) => {
                logging::error_file_async(format!(
                    "Failed to yahoo::profile::visit because {:?}",
                    why
                ));
                continue;
            }
        };

        if year != profile.year || quarter != profile.quarter {
            logging::warn_file_async(format!(
                "the year or quarter retrieved from Yahoo is inconsistent with the current one. current year:{} ,quarter:{} {:#?}",
                year, quarter, profile
            ));
            continue;
        }

        let fs = table::financial_statement::FinancialStatement::from(profile);
        if let Err(why) = fs.clone().upsert().await {
            logging::error_file_async(format!("{:?}", why));
            continue;
        }

        logging::info_file_async(format!(
            "financial_statement upsert executed successfully. \r\n{:#?}",
            fs
        ));

        success_update_count += 1;
    }

    if success_update_count > 0 {
        table::stock::Stock::update_last_eps().await?;
        let estimate_date_config =
            table::config::Config::new("estimate-date".to_string(), "".to_string());

        let date = estimate_date_config.get_val_naive_date().await?;
        // 計算便宜、合理、昂貴價的估算
        calculation::estimated_price::calculate_estimated_price(date).await?;
        logging::info_file_async("季度財報更新重新計算便宜、合理、昂貴價的估算結束".to_string());
    }

    nosql::redis::CLIENT
        .set(cache_key, true, 60 * 60 * 24 * 7)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::internal::cache::SHARE;
    use crate::internal::logging;

    use super::*;

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
