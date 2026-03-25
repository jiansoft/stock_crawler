//! 台股季度財報補欄位流程。
//!
//! 此模組負責找出指定季度中 `ROE`、`ROA` 或每股淨值為零的財報資料，
//! 再到 Yahoo 財經補抓對應欄位並回寫資料庫。
//!
//! 目標季度必須與季 EPS 主流程一致，因此同樣使用
//! [`crate::util::datetime::latest_published_quarter_for_listed_and_otc`]
//! 依上市/上櫃公司季報法定申報截止日判定，不再使用固定天數回推。

use anyhow::Result;
use chrono::Local;

use crate::{
    backfill::financial_statement::update_roe_and_roa_for_zero_values, calculation, crawler::yahoo,
    database::table, logging, nosql, util::map::Keyable,
};

/// 補齊最新應已公告季度財報中缺漏的 ROE、ROA 與每股淨值欄位。
///
/// 本流程會先找出目標季度中數值為零的財報，再逐筆到 Yahoo 財經抓取對應資料，
/// 回寫 `financial_statement` 後，重新同步 `stocks` 表上的最新一季/近四季數據，
/// 並觸發估值重算。
pub async fn execute() -> Result<()> {
    let target_report =
        crate::util::datetime::latest_published_quarter_for_listed_and_otc(Local::now());
    let quarter = target_report.quarter.to_string();
    let fss = table::financial_statement::fetch_roe_or_roa_equal_to_zero(
        Some(target_report.year),
        Some(target_report.quarter),
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

    if let Err(why) = update_roe_and_roa_for_zero_values(Some(target_report.quarter)).await {
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
