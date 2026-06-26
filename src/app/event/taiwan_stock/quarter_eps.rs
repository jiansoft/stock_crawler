//! 台股季 EPS 更新事件。
//!
//! 此模組負責抓取「目前依法定申報期限，理論上應已完整公告」的正式季度 EPS，
//! 並在下一季季報所屬季度結束後，額外預抓已提早公告的下一季 EPS，
//! 再一併寫回 `financial_statement` 資料表。
//!
//! 這裡不再使用「目前時間減去固定天數」的猜測方式，而是改由
//! [`util::datetime::eps_report_quarter_targets_for_listed_and_otc`] 根據
//! 上市/上櫃公司的季報申報截止日與「季末隔天起即可預抓」規則推導目標季度清單，
//! 避免在截止日前過早切換主季度，同時也能提早收錄已公告的下一季資料。

use std::collections::HashSet;

use crate::{app::backfill::acl::FinancialStatementAclMapper, core::declare::StockExchangeMarket, core::util::{self, datetime::ReportQuarter}, domain::registry::repository::StockRepository, infra::crawler::twse, infra::database::repository::financial::PgFinancialRepository, infra::database::repository::stock::PgStockRepository};
use anyhow::Result;
use chrono::Local;
use scopeguard::defer;

/// <summary>
/// 執行台股季 EPS 更新流程。
/// </summary>
pub async fn execute() -> Result<()> {
    tracing::info!("更新台股季度財報開始");
    defer! {
       tracing::info!("更新台股季度財報結束");
    }

    for target_report in util::datetime::eps_report_quarter_targets_for_listed_and_otc(Local::now())
    {
        if let Err(why) = process_target_report(target_report).await {
            tracing::error!("Failed to process quarterly EPS target {} {} because {:?}",
                target_report.year, target_report.quarter, why);
        }
    }

    Ok(())
}

/// <summary>
/// 處理單一目標季度的季 EPS 抓取流程。
/// </summary>
async fn process_target_report(target_report: ReportQuarter) -> Result<()> {
    let quarter = target_report.quarter.to_string();
    let stock_repo = PgStockRepository::new();

    // 取得指定季度中，缺漏財務報表的證券代號清單 (Vec<String>)
    let without_fs_stocks = stock_repo
        .fetch_stocks_without_financial_statement(target_report.year, &quarter)
        .await?;
    let without_financial_stocks: HashSet<String> = without_fs_stocks.into_iter().collect();

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
            tracing::error!("Failed to update quarterly EPS for {} {} {} because {:?}",
                market, target_report.year, target_report.quarter, why);
            continue;
        }
    }

    Ok(())
}

/// <summary>
/// 依市場抓取指定季度的 EPS，並寫回 `financial_statement`。
/// </summary>
/// <param name="market">目標市場類型 (上市或上櫃)</param>
/// <param name="year">目標財報年度</param>
/// <param name="quarter">目標財報季度</param>
/// <param name="without_financial_stocks">尚未寫入該季度財報的股票代號集合</param>
async fn process_eps(
    market: StockExchangeMarket,
    year: i32,
    quarter: crate::core::declare::Quarter,
    without_financial_stocks: &HashSet<String>,
) -> Result<()> {
    use crate::domain::financial::repository::FinancialRepository;

    let financial_repo = PgFinancialRepository::new();
    let eps = twse::eps::visit(market, year, quarter).await?;

    for mut e in eps {
        if !without_financial_stocks.contains(&e.stock_symbol) {
            // 不在清單內代表該股票的目標季度資料已收錄。
            continue;
        }

        if e.quarter != crate::core::declare::Quarter::Q1 {
            // Q2~Q4 在來源站通常是累計 EPS，需扣掉前面季度後還原為單季值。
            let smaller_quarters = quarter.smaller_quarters();
            let before_eps = financial_repo
                .fetch_cumulative_eps(&e.stock_symbol, year, smaller_quarters)
                .await?;
            e.earnings_per_share -= before_eps;
        }

        // 透過防腐層轉譯器，直接將 DTO 轉為領域實體
        let fs = FinancialStatementAclMapper::from_eps(e);

        if let Err(why) = financial_repo.save_earnings_per_share(&fs).await {
            tracing::error!("{:?}", why);
        }

        tracing::debug!("financial_statement earnings_per_share executed successfully. \r\n{:#?}",
            fs);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::core::declare::Quarter;
    use crate::infra::cache::SHARE;
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn test_execute() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 execute");

        match execute().await {
            Ok(_) => {}
            Err(why) => {
                tracing::debug!("Failed to execute because {:?}", why);
            }
        }

        tracing::debug!("結束 execute");
    }

    #[tokio::test]
    async fn test_process_eps() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        tracing::info!("開始 process_eps");
        use crate::domain::registry::repository::StockRepository;
        let stock_repo = PgStockRepository::new();
        let without_financial_stocks = match stock_repo
            .fetch_stocks_without_financial_statement(2023, Quarter::Q4.to_string().as_str())
            .await
        {
            Ok(stocks) => stocks.into_iter().collect::<HashSet<String>>(),
            Err(why) => {
                tracing::debug!("Failed to fetch stocks without financial statement: {:?}",
                    why);
                return;
            }
        };
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
                tracing::debug!("Failed to process_eps because: {:?}", why);
            }
        }

        tracing::info!("結束 process_eps");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
