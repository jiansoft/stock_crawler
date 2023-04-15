use crate::{
    internal::cache_share::CACHE_SHARE,
    internal::database::model::revenue, internal::*, logging,
};
use chrono::{Datelike, FixedOffset};
use concat_string::concat_string;
use rust_decimal::Decimal;
use scraper::{Html, Selector};
use std::str::FromStr;

/// 下載月營收
pub async fn visit(date_time: chrono::DateTime<FixedOffset>) -> Option<Vec<revenue::Entity>> {
    let year = date_time.year();
    let republic_of_china_era = year - 1911;
    let month = date_time.month();

    let mut new_entity = Vec::with_capacity(1024);

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

            if let Some(r) = download_revenue(url, year, month).await {
                new_entity.extend(r);
            }
        }
    }

    if new_entity.is_empty() {
        None
    } else {
        Some(new_entity)
    }
}

/// 下載月營收
async fn download_revenue(url: String, year: i32, month: u32) -> Option<Vec<revenue::Entity>> {
    logging::info_file_async(format!("visit url:{}", url));

    let text = match util::http::request_get_use_big5(&url).await {
        Err(_) => {
            return None;
        }
        Ok(t) => t,
    };

    let mut new_entity = Vec::with_capacity(1024);
    if let Ok(selector) = Selector::parse("body > center > center > table > tbody > tr > td > table > tbody > tr > td > table > tbody > tr") {
        let date = ((year * 100) + month as i32) as i64;
        let document = Html::parse_document(text.as_str());
        for (_tr_count, node) in document.select(&selector).enumerate() {
            let tds: Vec<&str> = node.text().clone().enumerate().map(|(_i, v)| v).collect();
            //println!("tds({}):{:?}",tds.len(),tds);
            if tds.len() != 11 {
                continue;
            }

            //
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
            0公司代號	1公司名稱	2當月營收	3上月營收	4去年當月營收	5上月比較增減(%) 6去年同月增減(%) 7當月累計營收 8去年累計營收 9前期比較增減(%)
            */

            e.monthly = Decimal::from_str(tds[2].replace([',', ' '], "").as_str()).unwrap_or_default();
            e.last_month = Decimal::from_str(tds[3].replace([',', ' '], "").as_str()).unwrap_or_default();
            e.last_year_this_month = Decimal::from_str(tds[4].replace([',', ' '], "").as_str()).unwrap_or_default();
            e.monthly_accumulated = Decimal::from_str(tds[7].replace([',', ' '], "").as_str()).unwrap_or_default();
            e.last_year_monthly_accumulated = Decimal::from_str(tds[8].replace([',', ' '], "").as_str()).unwrap_or_default();
            e.compared_with_last_month = Decimal::from_str(tds[5].replace([',', ' '], "").as_str()).unwrap_or_default();
            e.compared_with_last_year_same_month = Decimal::from_str(tds[6].replace([',', ' '], "").as_str()).unwrap_or_default();
            e.accumulated_compared_with_last_year = Decimal::from_str(tds[9].replace([',', ' '], "").as_str()).unwrap_or_default();

            new_entity.push(e);
        }
    }

    Some(new_entity)
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
