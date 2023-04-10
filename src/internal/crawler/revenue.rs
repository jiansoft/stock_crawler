use crate::{
    internal::cache_share::CACHE_SHARE, internal::database::model,
    internal::database::model::revenue, internal::*, logging,
};
use chrono::{Datelike, FixedOffset, Local};
use concat_string::concat_string;
use rust_decimal::Decimal;
use scraper::{Html, Selector};
use std::str::FromStr;

/// 下載月營收
pub async fn visit(date_time: chrono::DateTime<FixedOffset>) {
    let year = date_time.year();
    let republic_of_china_era = year - 1911;
    let month = date_time.month();
    let mut count: usize = 0;
    for market in ["sii", "otc"] {
        for i in 0..2 {
            let url = concat_string!(
                "https://mops.twse.com.tw/nas/t21/",
                market,
                "/t21sc03_",
                republic_of_china_era.to_string(),
                "_",
                month.to_string(),
                "_",
                i.to_string(),
                ".html"
            );

            count += download_revenue(url, year, month).await;
        }
    }

    if count > 0 {
        revenue::rebuild_revenue_last_date().await;
    }
}

/// 下載月營收
async fn download_revenue(url: String, year: i32, month: u32) -> usize {
    logging::info_file_async(format!("visit url:{}", url));

    let t = match util::http::request_get_use_big5(&url).await {
        Err(_) => {
            return 0;
        }
        Ok(t) => t,
    };

    let mut new_entity = Vec::with_capacity(1024);

    if let Ok(selector) = Selector::parse("body > center > center > table > tbody > tr > td > table > tbody > tr > td > table > tbody > tr") {
        let date = ((year * 100) + month as i32) as i64;
        let document = Html::parse_document(t.as_str());
        for (_tr_count, node) in document.select(&selector).enumerate() {
            let tds: Vec<&str> = node.text().clone().enumerate().map(|(_i, v)| v).collect();

            if tds.len() != 11 {
                continue;
            }

            //println!("tds({}):{:?}",tds.len(),tds);
            let mut e = revenue::Entity::new();
            e.date = date;
            e.security_code = tds[0].to_string();

            // 檢查是否收錄過
            if let Ok(last_revenues) = CACHE_SHARE.last_revenues.read() {
                if let Some(last_revenue_date) = last_revenues.get(&date) {
                    if last_revenue_date.contains_key(&e.security_code.to_string()) {
                        //println!("已收:{} {}-{}", &e.security_code, year, month);
                        continue
                    }
                }
            }

            /*
            0公司代號	1公司名稱	2當月營收	3上月營收	4去年當月營收	5上月比較增減(%)
            6去年同月增減(%)	7當月累計營收	8去年累計營收	9前期比較增減(%)
            */

            e.monthly = Decimal::from_str(tds[2].replace([',', ' '], "").as_str()).unwrap_or(Decimal::ZERO);
            e.last_month = Decimal::from_str(tds[3].replace([',', ' '], "").as_str()).unwrap_or(Decimal::ZERO);
            e.last_year_this_month = Decimal::from_str(tds[4].replace([',', ' '], "").as_str()).unwrap_or(Decimal::ZERO);
            e.monthly_accumulated = Decimal::from_str(tds[7].replace([',', ' '], "").as_str()).unwrap_or(Decimal::ZERO);
            e.last_year_monthly_accumulated = Decimal::from_str(tds[8].replace([',', ' '], "").as_str()).unwrap_or(Decimal::ZERO);
            e.compared_with_last_month = Decimal::from_str(tds[5].replace([',', ' '], "").as_str()).unwrap_or(Decimal::ZERO);
            e.compared_with_last_year_same_month = Decimal::from_str(tds[6].replace([',', ' '], "").as_str()).unwrap_or(Decimal::ZERO);
            e.accumulated_compared_with_last_year = Decimal::from_str(tds[9].replace([',', ' '], "").as_str()).unwrap_or(Decimal::ZERO);
            e.create_time = Local::now();
            new_entity.push(e);
        }
    }

    let size = new_entity.len();
    for mut e in new_entity {
        let mut stock = model::stock::Entity::new();
        stock.stock_symbol = e.security_code.to_string();
        if let Ok((lowest_price, avg_price, highest_price)) = stock
            .lowest_avg_highest_price_by_year_and_month(year, month as i32)
            .await
        {
            e.lowest_price = lowest_price;
            e.avg_price = avg_price;
            e.highest_price = highest_price;
        }

        match e.upsert().await {
            Ok(_) => {
                if let Ok(mut last_revenues) = CACHE_SHARE.last_revenues.write() {
                    if let Some(last_revenue_date) = last_revenues.get_mut(&e.date) {
                        last_revenue_date
                            .entry(e.security_code.to_string())
                            .or_insert(e.clone());
                    }
                }

                let name = match CACHE_SHARE.stocks.read() {
                    Ok(stocks) => {
                        if let Some(stock) = stocks.get(e.security_code.as_str()) {
                            stock.name.to_string()
                        } else {
                            "no name".to_string()
                        }
                    }
                    Err(why) => {
                        logging::error_file_async(format!("because {:?}", why));
                        "no name".to_string()
                    }
                };

                logging::info_file_async(
                    format!(
                        "公司代號:{}  公司名稱:{} 當月營收:{} 上月營收:{} 去年當月營收:{} 月均價:{} 最低價:{} 最高價:{}",
                        e.security_code,
                        name,
                        e.monthly,
                        e.last_month,
                        e.last_year_this_month,
                        e.avg_price,
                        e.lowest_price,
                        e.highest_price))
            }
            Err(why) => {
                logging::error_file_async(format!("because {:?}", why));
            }
        }
    }

    size
}

#[cfg(test)]
mod tests {

    use chrono::prelude::*;
    use chrono::{Duration, Local};
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    async fn test_visit() {
        dotenv::dotenv().ok();
        CACHE_SHARE.load().await;
        let _now = Local::now();

        let naive_datetime = NaiveDate::from_ymd_opt(2023, 3, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();

        let last_month = naive_datetime - Duration::minutes(1);

        let timezone = FixedOffset::east_opt(8 * 60 * 60).unwrap();
        let last_month_timezone = DateTime::<FixedOffset>::from_local(last_month, timezone);
        println!("last_month_timezone:{:?}", last_month_timezone);
        visit(last_month_timezone).await;
    }
}
