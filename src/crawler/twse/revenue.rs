use anyhow::{anyhow, Result};
use chrono::{Datelike, FixedOffset};
use scraper::{Html, Selector};

use crate::{cache::SHARE, crawler::twse, database::table::revenue, util};

/// 下載月營收
pub async fn visit(date_time: chrono::DateTime<FixedOffset>) -> Result<Vec<revenue::Revenue>> {
    let year = date_time.year();
    let republic_of_china_era = util::datetime::gregorian_year_to_roc_year(year);
    let month = date_time.month();
    let mut revenues = Vec::with_capacity(1024);

    for market in ["sii", "otc"].iter() {
        for i in 0..2 {
            let url = format!(
                "https://mopsov.{}/nas/t21/{}/t21sc03_{}_{}_{}.html",
                twse::HOST,
                market,
                republic_of_china_era,
                month,
                i
            );

            if let Ok(r) = download_revenue(url, year, month).await {
                revenues.extend(r);
            }
        }
    }

    Ok(revenues)
}

/// 下載月營收
async fn download_revenue(url: String, year: i32, month: u32) -> Result<Vec<revenue::Revenue>> {
    let text = util::http::get_use_big5(&url).await?;
    let mut revenues = Vec::with_capacity(1024);

    // 改用更具彈性的選擇器：先抓取所有 tr
    let tr_selector = Selector::parse("tr")
        .map_err(|why| anyhow!("Failed to Selector::parse tr because: {:?}", why))?;
    // 用於選取 tr 內部的 td
    let td_selector = Selector::parse("td")
        .map_err(|why| anyhow!("Failed to Selector::parse td because: {:?}", why))?;

    let date = ((year * 100) + month as i32) as i64;
    let document = Html::parse_document(text.as_str());

    for node in document.select(&tr_selector) {
        // 1. 先取得儲存格迭代器
        let mut cell_nodes = node.select(&td_selector);

        // 2. 優先處理第一欄 (公司代號)
        let first_cell_text = match cell_nodes.next() {
            Some(td) => td.text().collect::<String>(),
            None => continue,
        };
        let code = first_cell_text.trim();

        // 3. 立即檢查：如果第一欄不是數字，直接跳過整行，不處理後續 td
        // 這能有效過濾掉標題列、說明文字或合計列，減少記憶體分配
        if code.is_empty() || !code.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        // 4. 只有符合條件的行，才去 collect 剩下的欄位
        let mut tds = Vec::with_capacity(11);
        tds.push(code.to_owned()); // 加入已處理好的第一欄

        tds.extend(cell_nodes.map(|td| td.text().collect::<String>().trim().to_owned()));

        // 5. 營收資料表格通常有 10-11 個欄位
        if tds.len() < 10 {
            continue;
        }

        // 檢查是否收錄過
        if SHARE.last_revenues_contains_key(date, &tds[0]) {
            continue;
        }

        let mut entity = revenue::Revenue::from(tds);
        entity.date = date;
        revenues.push(entity);
    }

    Ok(revenues)
}

#[cfg(test)]
mod tests {
    use chrono::prelude::*;
    use chrono::{Local, TimeDelta};
    use std::time::Duration;

    use crate::logging;

    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        let _now = Local::now();

        let naive_datetime = NaiveDate::from_ymd_opt(2026, 2, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();

        let last_month = naive_datetime - TimeDelta::try_minutes(1).unwrap();

        let timezone = FixedOffset::east_opt(8 * 60 * 60).unwrap();
        let last_month_timezone = timezone.from_local_datetime(&last_month).unwrap();
        println!("last_month_timezone:{:?}", last_month_timezone);
        match visit(last_month_timezone).await {
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because: {:?}", why));
            }
            Ok(list) => {
                logging::debug_file_async(format!("data:{:#?}", list));
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
