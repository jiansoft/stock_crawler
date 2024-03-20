use anyhow::Result;
use chrono::{Datelike, Local, TimeDelta};

use crate::{
    backfill::financial_statement::update_roe_and_roa_for_zero_values, calculation, crawler::yahoo,
    database::table, declare::Quarter, logging, nosql, util::map::Keyable,
};

/// 將季度財報 ROE為零的數據，到雅虎財經下載後回寫到 financial_statement 表
pub async fn execute() -> Result<()> {
    let now = Local::now();
    let previous_quarter = now - TimeDelta::try_days(130).unwrap();
    let year = previous_quarter.year();
    let previous_quarter = Quarter::from_month(now.month()).unwrap().previous();
    let quarter = previous_quarter.to_string();
    let fss = table::financial_statement::fetch_roe_or_roa_equal_to_zero(
        Some(year),
        Some(previous_quarter),
    )
    .await?;
    let mut success_count = 0;

    for fs in fss {
        let cache_key = fs.key_with_prefix();
        let is_jump = nosql::redis::CLIENT.get_bool(&cache_key).await?;

        if is_jump {
            continue;
        }

        let profile = match yahoo::profile::visit(&fs.security_code).await {
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

        logging::debug_file_async(format!(
            "financial_statement upsert executed successfully. \r\n{:#?}",
            fs
        ));

        nosql::redis::CLIENT
            .set(cache_key, true, 60 * 60 * 24 * 7)
            .await?;

        success_count += 1;
    }

    if let Err(why) = update_roe_and_roa_for_zero_values(Some(previous_quarter)).await {
        logging::error_file_async(format!("{:#?}", why));
    }

    if success_count > 0 {
        table::stock::Stock::update_eps_and_roe().await?;
        let estimate_date_config =
            table::config::Config::new("estimate-date".to_string(), "".to_string());
        let date = estimate_date_config.get_val_naive_date().await?;
        // 計算便宜、合理、昂貴價的估算
        calculation::estimated_price::calculate_estimated_price(date).await?;
        logging::info_file_async("季度財報更新重新計算便宜、合理、昂貴價的估算結束".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{cache::SHARE, logging};

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
