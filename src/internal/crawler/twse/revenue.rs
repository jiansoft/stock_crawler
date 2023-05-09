use crate::internal::{cache::SHARE, database::model::revenue, logging, util};
use chrono::{Datelike, FixedOffset};
use scraper::{Html, Selector};

/// 下載月營收
pub async fn visit(date_time: chrono::DateTime<FixedOffset>) -> Option<Vec<revenue::Entity>> {
    let year = date_time.year();
    let republic_of_china_era = year - 1911;
    let month = date_time.month();
    let mut new_entity = Vec::with_capacity(1024);

    for market in ["sii", "otc"].iter() {
        for i in 0..2 {
            let url = format!(
                "https://mops.twse.com.tw/nas/t21/{}/t21sc03_{}_{}_{}.html",
                market, republic_of_china_era, month, i
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

    let text = util::http::request_get_use_big5(&url).await.ok()?;
    let mut new_entity = Vec::with_capacity(1024);
    let selector = Selector::parse("body > center > center > table > tbody > tr > td > table > tbody > tr > td > table > tbody > tr").ok()?;
    let date = ((year * 100) + month as i32) as i64;
    let document = Html::parse_document(text.as_str());
    for node in document.select(&selector) {
        let tds: Vec<String> = node.text().map(|v| v.to_string()).collect();

        if tds.len() != 11 {
            continue;
        }

        // 檢查是否收錄過
        if let Ok(last_revenues) = SHARE.last_revenues.read() {
            if let Some(last_revenue_date) = last_revenues.get(&date) {
                if last_revenue_date.contains_key(&tds[0]) {
                    //println!("已收:{} {}-{}", &e.security_code, year, month);
                    continue;
                }
            }
        }

        let mut entity = revenue::Entity::from(tds);
        entity.date = date;
        new_entity.push(entity);
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
        SHARE.load().await;
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
