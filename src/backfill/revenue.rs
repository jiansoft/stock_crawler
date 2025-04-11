use anyhow::Result;
use chrono::{Datelike, FixedOffset, Local, NaiveDate, TimeDelta, TimeZone};
use futures::{stream, StreamExt};
use scopeguard::defer;
use crate::{
    cache::SHARE,
    crawler::twse,
    database::{table, table::revenue},
    logging, util,
};

/// 調用  twse API 取得台股月營收
pub async fn execute() -> Result<()> {
    logging::info_file_async("更新台股月營收開始");
    defer! {
       logging::info_file_async("更新台股月營收結束");
    }
    let now = Local::now();
    let naive_datetime = NaiveDate::from_ymd_opt(now.year(), 3, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let last_month = naive_datetime - TimeDelta::try_minutes(1).unwrap();
    let timezone = FixedOffset::east_opt(8 * 60 * 60).unwrap();
    let last_month_timezone = timezone.from_local_datetime(&last_month).unwrap();
    let year = last_month_timezone.year();
    let month = last_month_timezone.month();
    let revenues = twse::revenue::visit(last_month_timezone).await?;

    stream::iter(revenues)
        .for_each_concurrent(util::concurrent_limit_16(), |r| async move {
            if let Err(why) = process_revenue(r, year, month as i32).await {
                logging::error_file_async(format!("Failed to process_revenue because {:?}", why));
            }
        })
        .await;

    revenue::rebuild_revenue_last_date().await?;

    Ok(())
}

pub(crate) async fn process_revenue(
    mut revenue: revenue::Revenue,
    year: i32,
    month: i32,
) -> Result<()> {
    if let Ok(dq) =
        table::daily_quote::fetch_monthly_stock_price_summary(&revenue.security_code, year, month)
            .await
    {
        revenue.lowest_price = dq.lowest_price;
        revenue.avg_price = dq.avg_price;
        revenue.highest_price = dq.highest_price;
    }

    revenue.upsert().await?;

    SHARE.set_last_revenues(revenue.clone());

    let name = match SHARE.get_stock(&revenue.security_code).await {
        None => String::from("-"),
        Some(s) => s.name.clone(),
    };

    logging::info_file_async(
        format!(
            "公司代號:{}  公司名稱:{} 當月營收:{} 上月營收:{} 去年當月營收:{} 月均價:{} 最低價:{} 最高價:{}",
            revenue.security_code,
            name,
            revenue.monthly,
            revenue.last_month,
            revenue.last_year_this_month,
            revenue.avg_price,
            revenue.lowest_price,
            revenue.highest_price));

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use crate::logging;

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
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
