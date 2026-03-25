//! 台股季度財報補欄位流程。
//!
//! 此模組負責找出指定季度中 `ROE`、`ROA` 或每股淨值為零的財報資料，
//! 再到 Yahoo 財經補抓對應欄位並回寫資料庫。
//!
//! 目標季度必須與季 EPS 主流程一致，因此同樣使用
//! [`crate::util::datetime::backfill_report_quarter_targets_for_listed_and_otc`]
//! 依上市/上櫃公司季報法定申報截止日與較保守的預抓視窗決定處理清單。

use anyhow::Result;
use chrono::Local;

use crate::{
    backfill::financial_statement::update_roe_and_roa_for_zero_values,
    calculation,
    crawler::yahoo,
    database::table,
    logging, nosql,
    util::{datetime::ReportQuarter, map::Keyable},
};

/// 補齊最新應已公告季度財報中缺漏的 ROE、ROA 與每股淨值欄位。
///
/// 本流程會先找出正式季度與預抓季度中數值為零的財報，再逐筆到 Yahoo 財經
/// 抓取對應資料，回寫 `financial_statement` 後，重新同步 `stocks` 表上的
/// 最新一季/近四季數據，並觸發估值重算。
pub async fn execute() -> Result<()> {
    let mut success_count = 0usize;

    for target_report in
        crate::util::datetime::backfill_report_quarter_targets_for_listed_and_otc(Local::now())
    {
        success_count += process_target_report(target_report).await?;

        if let Err(why) = update_roe_and_roa_for_zero_values(Some(target_report.quarter)).await {
            logging::error_file_async(format!("{:#?}", why));
        }
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

/// 補齊單一目標季度內缺漏的 Yahoo 財務欄位。
async fn process_target_report(target_report: ReportQuarter) -> Result<usize> {
    let quarter = target_report.quarter.to_string();
    let fss = table::financial_statement::fetch_roe_or_roa_equal_to_zero(
        Some(target_report.year),
        Some(target_report.quarter),
    )
    .await?;
    let mut success_count = 0usize;

    for fs in fss {
        let cache_key = fs.key_with_prefix();
        let profile_skip_cache_key = yahoo::profile::no_valid_data_cache_key(&fs.security_code);
        let is_jump = nosql::redis::CLIENT.get_bool(&cache_key).await?;
        let is_profile_skip = nosql::redis::CLIENT
            .get_bool(&profile_skip_cache_key)
            .await?;

        if is_jump || is_profile_skip {
            continue;
        }

        let profile = match yahoo::profile::visit(&fs.security_code).await {
            Ok(profile) => profile,
            Err(why) => {
                if yahoo::profile::is_no_valid_data_error(&why) {
                    if let Err(cache_err) = nosql::redis::CLIENT
                        .set(
                            &profile_skip_cache_key,
                            true,
                            yahoo::profile::NO_VALID_DATA_CACHE_TTL_SECONDS,
                        )
                        .await
                    {
                        logging::error_file_async(format!(
                            "Failed to cache yahoo::profile no-valid-data skip for {} because {:?}",
                            fs.security_code, cache_err
                        ));
                    }
                    logging::warn_file_async(format!(
                        "Skip yahoo::profile::visit for {} because {}",
                        fs.security_code, why
                    ));
                } else {
                    logging::error_file_async(format!(
                        "Failed to yahoo::profile::visit for {} because {}",
                        fs.security_code, why
                    ));
                }
                continue;
            }
        };

        if target_report.year != profile.year || quarter != profile.quarter {
            logging::warn_file_async(format!(
                "the year or quarter retrieved from Yahoo is inconsistent with the current one. current year:{} ,quarter:{} {:#?}",
                target_report.year, quarter, profile
            ));
            //continue;
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

    Ok(success_count)
}

#[cfg(test)]
mod tests {
    use crate::{cache::SHARE, logging};

    use super::*;

    #[tokio::test]
    #[ignore]
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
