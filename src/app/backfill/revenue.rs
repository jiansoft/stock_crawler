use crate::{
    app::backfill::acl::{RevenueAclMapper, UpdateRevenueCommand},
    core::logging,
    core::util,
    infra::cache::SHARE,
    infra::crawler::twse,
};
use anyhow::Result;
use chrono::{Datelike, FixedOffset, Local, NaiveDate, TimeDelta, TimeZone};
use futures::{StreamExt, stream};
use scopeguard::defer;

/// 調用  twse API 取得台股月營收
pub async fn execute() -> Result<()> {
    logging::info_file_async("更新台股月營收開始");
    defer! {
        logging::info_file_async("更新台股月營收結束");
    }

    let now = Local::now();
    let naive_datetime = NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let last_month = naive_datetime - TimeDelta::try_minutes(1).unwrap();
    let timezone = FixedOffset::east_opt(8 * 60 * 60).unwrap();
    let last_month_timezone = timezone.from_local_datetime(&last_month).unwrap();

    process_revenues(last_month_timezone).await
}

async fn process_revenues(last_month_timezone: chrono::DateTime<FixedOffset>) -> Result<()> {
    use crate::domain::financial::repository::FinancialRepository;
    use crate::infra::database::repository::financial::PgFinancialRepository;

    let financial_repo = PgFinancialRepository::new();
    let year = last_month_timezone.year();
    let month = last_month_timezone.month();

    let revenues = twse::revenue::visit(last_month_timezone).await?;
    let cmds: Vec<UpdateRevenueCommand> = revenues.iter().map(RevenueAclMapper::from_dto).collect();

    stream::iter(cmds)
        .for_each_concurrent(util::concurrent_limit_16(), |cmd| async move {
            if let Err(why) = process_revenue(cmd, year, month as i32).await {
                logging::error_file_async(format!("Failed to process_revenue because {:?}", why));
            }
        })
        .await;

    financial_repo.rebuild_revenue_last_date().await?;

    Ok(())
}

pub(crate) async fn process_revenue(
    cmd: UpdateRevenueCommand,
    year: i32,
    month: i32,
) -> Result<()> {
    use crate::domain::financial::entity::MonthlyRevenue as DomainMonthlyRevenue;
    use crate::domain::financial::repository::FinancialRepository;
    use crate::infra::database::repository::financial::PgFinancialRepository;

    use crate::domain::quote::repository::QuoteRepository;
    use crate::infra::database::repository::quote::PgQuoteRepository;

    let financial_repo = PgFinancialRepository::new();
    let quote_repo = PgQuoteRepository::new();
    let mut table_entity = RevenueAclMapper::from_command(&cmd);

    if let Ok(Some((lowest_price, avg_price, highest_price))) = quote_repo
        .fetch_monthly_stock_price_summary(&cmd.symbol, year, month)
        .await
    {
        table_entity.lowest_price = lowest_price;
        table_entity.avg_price = avg_price;
        table_entity.highest_price = highest_price;
    }

    // 轉成領域實體進行儲存
    let domain_entity = DomainMonthlyRevenue::from(table_entity.clone());
    financial_repo.save_monthly_revenue(&domain_entity).await?;

    // 快取維持使用原 Table 實體以維持相容性
    SHARE.set_last_revenues(table_entity.clone());

    let name = match SHARE.get_stock(&cmd.symbol).await {
        None => String::from("-"),
        Some(s) => s.name().to_string(),
    };

    logging::info_file_async(format!(
        "公司代號:{}  公司名稱:{} 當月營收:{} 上月營收:{} 去年當月營收:{} 月均價:{} 最低價:{} 最高價:{}",
        cmd.symbol,
        name,
        cmd.monthly,
        cmd.last_month,
        cmd.last_year_this_month,
        table_entity.avg_price,
        table_entity.lowest_price,
        table_entity.highest_price
    ));

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::core::logging;
    use std::time::Duration;

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
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    #[tokio::test]
    #[ignore]
    async fn test_process_revenues() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 test_process_revenues".to_string());

        let naive_datetime = NaiveDate::from_ymd_opt(2025, 4, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let timezone = FixedOffset::east_opt(8 * 60 * 60).unwrap();
        let month_timezone = timezone.from_local_datetime(&naive_datetime).unwrap();

        match process_revenues(month_timezone).await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to test_process_revenues because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 test_process_revenues".to_string());
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
