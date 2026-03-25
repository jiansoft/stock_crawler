//! 台股季 EPS 更新事件。
//!
//! 此模組負責抓取「目前依法定申報期限，理論上應已完整公告」的正式季度 EPS，
//! 並在下一季季報所屬季度結束後，額外預抓已提早公告的下一季 EPS，
//! 再一併寫回 `financial_statement` 資料表。
//!
//! 這裡不再使用「目前時間減去固定天數」的猜測方式，而是改由
//! [`crate::util::datetime::eps_report_quarter_targets_for_listed_and_otc`] 根據
//! 上市/上櫃公司的季報申報截止日與「季末隔天起即可預抓」規則推導目標季度清單，
//! 避免在截止日前過早切換主季度，同時也能提早收錄已公告的下一季資料。

use std::collections::HashMap;

use crate::{
    crawler::twse,
    database::table::{self, financial_statement, stock::Stock},
    declare::StockExchangeMarket,
    logging,
    util::{self, datetime::ReportQuarter},
};
use anyhow::Result;
use chrono::Local;
use scopeguard::defer;

/// 執行台股季 EPS 更新流程。
///
/// 流程分為三個步驟：
///
/// 1. 依上市/上櫃公司季報法定申報截止日與季末時點，計算正式季度與可能的預抓季度清單。
/// 2. 查出各季度下尚未寫入 `financial_statement` 的股票。
/// 3. 分別向上市、上櫃市場抓取 EPS，必要時將累計 EPS 轉回單季 EPS 後回寫資料庫。
pub async fn execute() -> Result<()> {
    logging::info_file_async("更新台股季度財報開始");
    defer! {
       logging::info_file_async("更新台股季度財報結束");
    }

    for target_report in util::datetime::eps_report_quarter_targets_for_listed_and_otc(Local::now())
    {
        if let Err(why) = process_target_report(target_report).await {
            logging::error_file_async(format!(
                "Failed to process quarterly EPS target {} {} because {:?}",
                target_report.year, target_report.quarter, why
            ));
        }
    }

    Ok(())
}

/// 處理單一目標季度的季 EPS 抓取流程。
async fn process_target_report(target_report: ReportQuarter) -> Result<()> {
    let quarter = target_report.quarter.to_string();
    let without_fs_stocks =
        table::stock::fetch_stocks_without_financial_statement(target_report.year, &quarter)
            .await?;
    let without_financial_stocks = util::map::vec_to_hashmap(without_fs_stocks);

    for market in [
        StockExchangeMarket::Listed,
        StockExchangeMarket::OverTheCounter,
    ] {
        if let Err(why) = process_eps(
            market,
            target_report.year,
            target_report.quarter,
            &without_financial_stocks,
        )
        .await
        {
            logging::error_file_async(format!(
                "Failed to update quarterly EPS for {} {} {} because {:?}",
                market, target_report.year, target_report.quarter, why
            ));
            continue;
        }
    }

    Ok(())
}

/// 依市場抓取指定季度的 EPS，並寫回 `financial_statement`。
///
/// 由於公開資訊觀測站提供的 Q2、Q3、Q4 EPS 多為「累計值」，因此本函式會在
/// `Q1` 以外的季度，先扣除同年度更早季度的累計 EPS，還原成單季 EPS 後再寫入。
///
/// # 參數
///
/// * `market` - 目標市場，目前只會傳入上市或上櫃
/// * `year` - 目標財報年度
/// * `quarter` - 目標財報季度
/// * `without_financial_stocks` - 尚未寫入該季度財報的股票集合
async fn process_eps(
    market: StockExchangeMarket,
    year: i32,
    quarter: crate::declare::Quarter,
    without_financial_stocks: &HashMap<String, Stock>,
) -> Result<()> {
    let eps = twse::eps::visit(market, year, quarter).await?;

    for mut e in eps {
        if !without_financial_stocks.contains_key(&e.stock_symbol) {
            // 不在清單內代表該股票的目標季度資料已收錄。
            continue;
        }

        if e.quarter != crate::declare::Quarter::Q1 {
            // Q2~Q4 在來源站通常是累計 EPS，需扣掉前面季度後還原為單季值。
            let smaller_quarters = quarter.smaller_quarters();
            let before_eps =
                financial_statement::fetch_cumulative_eps(&e.stock_symbol, year, smaller_quarters)
                    .await?;
            e.earnings_per_share -= before_eps;
        }

        let fs = table::financial_statement::FinancialStatement::from(e);

        if let Err(why) = fs.upsert_earnings_per_share().await {
            logging::error_file_async(format!("{:?}", why));
        }

        logging::debug_file_async(format!(
            "financial_statement earnings_per_share executed successfully. \r\n{:#?}",
            fs
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::cache::SHARE;
    use crate::declare::Quarter;
    use std::time::Duration;

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

    #[tokio::test]
    async fn test_process_eps() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::info_file_async("開始 process_eps".to_string());
        //let now = Local::now();
        let without_financial_stocks = table::stock::fetch_stocks_without_financial_statement(
            2024,
            Quarter::Q1.to_string().as_str(),
        )
        .await
        .unwrap();
        let without_financial_stocks = util::map::vec_to_hashmap(without_financial_stocks);
        //dbg!(without_financial_stocks);
        match process_eps(
            StockExchangeMarket::Listed,
            2023,
            Quarter::Q4,
            &without_financial_stocks,
        )
        .await
        {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to process_eps because: {:?}", why));
            }
        }

        logging::info_file_async("結束 process_eps".to_string());
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
