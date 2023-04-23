use crate::{
    internal::{
        cache::SHARE,
        crawler::twse,
        database::model,
        logging
    }
};
use anyhow::*;
use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDate};
use core::result::Result::Ok;

/// 調用  twse API 取得台股月營收
pub async fn execute() -> Result<()> {
    let now = Local::now();
    let naive_datetime = NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let last_month = naive_datetime - chrono::Duration::minutes(1);
    let timezone = FixedOffset::east_opt(8 * 60 * 60).unwrap();
    let last_month_timezone = DateTime::<FixedOffset>::from_local(last_month, timezone);
    let results = match twse::revenue::visit(last_month_timezone).await {
        None => return Ok(()),
        Some(results) => {
            if results.is_empty() {
                return Ok(());
            }
            results
        }
    };

    let year = last_month_timezone.year();
    let month = last_month_timezone.month();

    for mut item in results {
        let mut stock = model::stock::Entity::new();
        stock.stock_symbol = item.security_code.to_string();
        if let Ok((lowest_price, avg_price, highest_price)) = stock
            .lowest_avg_highest_price_by_year_and_month(year, month as i32)
            .await
        {
            item.lowest_price = lowest_price;
            item.avg_price = avg_price;
            item.highest_price = highest_price;
        }

        if let Err(why) = item.upsert().await {
            logging::error_file_async(format!("Failed to item.upsert because {:?}", why));
            continue;
        }

        if let Ok(mut last_revenues) = SHARE.last_revenues.write() {
            if let Some(last_revenue_date) = last_revenues.get_mut(&item.date) {
                last_revenue_date
                    .entry(item.security_code.to_string())
                    .or_insert(item.clone());
            }
        }

        let name = SHARE
            .stocks
            .read()
            .map(|stocks| {
                stocks
                    .get(item.security_code.as_str())
                    .map_or("no name".to_string(), |stock| stock.name.to_string())
            })
            .unwrap_or_else(|why| {
                logging::error_file_async(format!("Failed to stocks.read because {:?}", why));
                "no name".to_string()
            });

        logging::info_file_async(
            format!(
                "公司代號:{}  公司名稱:{} 當月營收:{} 上月營收:{} 去年當月營收:{} 月均價:{} 最低價:{} 最高價:{}",
                item.security_code,
                name,
                item.monthly,
                item.last_month,
                item.last_year_this_month,
                item.avg_price,
                item.lowest_price,
                item.highest_price))
    }

    model::revenue::rebuild_revenue_last_date().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::logging;

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
