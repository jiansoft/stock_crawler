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
use futures::StreamExt;

use crate::{
    app::backfill::financial_statement::update_roe_and_roa_for_zero_values,
    app::calculation,
    core::logging,
    core::util::{datetime::ReportQuarter, map::Keyable},
    infra::crawler::yahoo,
    infra::database::table,
};

/// 補齊最新應已公告季度財報中缺漏的 ROE、ROA 與每股淨值欄位。
///
/// 本流程會先找出正式季度與預抓季度中數值為零的財報，再逐筆到 Yahoo 財經
/// 抓取對應資料，回寫 `financial_statement` 後，重新同步 `stocks` 表上的
/// 最新一季/近四季數據，並觸發估值重算。
pub async fn execute() -> Result<()> {
    let mut success_count = 0usize;

    for target_report in
        crate::core::util::datetime::backfill_report_quarter_targets_for_listed_and_otc(Local::now())
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
///
/// 使用 `futures::stream` 併發呼叫 Yahoo 財經爬取 Profile 資料，
/// 爬取完成後再一次批量 Upsert 入庫，大幅減少資料庫連線延遲，並降低 Yahoo 連線之 sequential 阻塞。
async fn process_target_report(target_report: ReportQuarter) -> Result<usize> {
    let quarter = target_report.quarter.to_string();
    // 找出該季度中 ROE 或 ROA 為 0 的財報清單
    let fss = table::financial_statement::fetch_roe_or_roa_equal_to_zero(
        Some(target_report.year),
        Some(target_report.quarter),
    )
    .await?;

    // 建立併發爬取的 async stream，並限制併發數量上限為 5
    let mut stream = futures::stream::iter(fss)
        .map(|fs| {
            let target_report: ReportQuarter = target_report;
            let quarter = quarter.clone();
            async move {
                let cache_key = fs.key_with_prefix();
                let profile_skip_cache_key = yahoo::profile::no_valid_data_cache_key(&fs.security_code);

                // 檢查 Redis 狀態，決定是否跳過（例如先前已處理過，或是已知無有效資料的股票）
                let is_jump = match crate::infra::nosql::redis::CLIENT.get_bool(&cache_key).await {
                    Ok(val) => val,
                    Err(why) => {
                        logging::error_file_async(format!("Redis error getting {}: {:?}", cache_key, why));
                        false
                    }
                };
                let is_profile_skip = match crate::infra::nosql::redis::CLIENT.get_bool(&profile_skip_cache_key).await {
                    Ok(val) => val,
                    Err(why) => {
                        logging::error_file_async(format!("Redis error getting {}: {:?}", profile_skip_cache_key, why));
                        false
                    }
                };

                if is_jump || is_profile_skip {
                    return None;
                }

                // 呼叫 Yahoo API 抓取 Profile 資訊
                let profile = match yahoo::profile::visit(&fs.security_code).await {
                    Ok(profile) => profile,
                    Err(why) => {
                        // 若為查無有效資料的錯誤，寫入 Redis 排除快取以避免重複查詢
                        if yahoo::profile::is_no_valid_data_error(&why) {
                            if let Err(cache_err) = crate::infra::nosql::redis::CLIENT
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
                        return None;
                    }
                };

                // 比對資料年份與季度是否與目標相符，若不符則記錄警告但依原邏輯仍繼續處理
                if target_report.year != profile.year || quarter != profile.quarter {
                    logging::warn_file_async(format!(
                        "the year or quarter retrieved from Yahoo is inconsistent with the current one. current year:{} ,quarter:{} {:#?}",
                        target_report.year, quarter, profile
                    ));
                }

                // 建立新的財報結構體
                let new_fs = table::financial_statement::FinancialStatement::from(profile);
                Some((new_fs, cache_key))
            }
        })
        .buffer_unordered(5);

    let mut scraped_statements = Vec::new();
    let mut keys_to_cache = Vec::new();

    // 收集所有成功爬取的結果
    while let Some(res) = stream.next().await {
        if let Some((new_fs, cache_key)) = res {
            scraped_statements.push(new_fs);
            keys_to_cache.push(cache_key);
        }
    }

    let success_count = scraped_statements.len();

    // 如果有成功爬取的財報資料，執行批量 Upsert 寫入資料庫，並更新對應的 Redis 快取
    if !scraped_statements.is_empty() {
        // 批量寫入資料庫
        table::financial_statement::FinancialStatement::batch_upsert(&scraped_statements).await?;
        logging::debug_file_async(format!(
            "financial_statement batch_upsert executed successfully for {} records.",
            scraped_statements.len()
        ));

        // 逐一更新 Redis 成功狀態快取
        for cache_key in keys_to_cache {
            if let Err(why) = crate::infra::nosql::redis::CLIENT
                .set(&cache_key, true, 60 * 60 * 24 * 7)
                .await
            {
                logging::error_file_async(format!(
                    "Failed to set redis cache key {} because {:?}",
                    cache_key, why
                ));
            }
        }
    }

    Ok(success_count)
}

#[cfg(test)]
mod tests {
    use crate::{core::logging, infra::cache::SHARE};

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
