use anyhow::Result;
use chrono::{Datelike, FixedOffset};
use scraper::{Html, Selector};

use crate::{
    core::util,
    infra::cache::SHARE,
    infra::crawler::{share::RevenueDto, twse},
};

/// 下載月營收
pub async fn visit(date_time: chrono::DateTime<FixedOffset>) -> Result<Vec<RevenueDto>> {
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
async fn download_revenue(url: String, year: i32, month: u32) -> Result<Vec<RevenueDto>> {
    let text = util::http::get_use_big5(&url).await?;
    let date = ((year * 100) + month as i32) as i64;
    let revenues = parse_revenue_html(&text, year, month)
        .into_iter()
        .filter(|dto| !SHARE.last_revenues_contains_key(date, &dto.stock_symbol))
        .collect();
    Ok(revenues)
}

/// 解析月營收 HTML 頁面，回傳所有可辨識的 `RevenueDto`（不含 SHARE 去重）。
///
/// 每列第一欄為公司代號（純數字），非數字的標題列、合計列均跳過。
/// 至少需要 10 欄才被視為有效資料列。
fn parse_revenue_html(html: &str, year: i32, month: u32) -> Vec<RevenueDto> {
    let date = ((year * 100) + month as i32) as i64;
    let mut revenues = Vec::with_capacity(1024);

    let Ok(tr_selector) = Selector::parse("tr") else {
        return revenues;
    };
    let Ok(td_selector) = Selector::parse("td") else {
        return revenues;
    };

    let document = Html::parse_document(html);

    for node in document.select(&tr_selector) {
        let mut cell_nodes = node.select(&td_selector);

        let first_cell_text = match cell_nodes.next() {
            Some(td) => td.text().collect::<String>(),
            None => continue,
        };
        let code = first_cell_text.trim();

        if code.is_empty() || !code.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        let mut tds = Vec::with_capacity(11);
        tds.push(code.to_owned());
        tds.extend(cell_nodes.map(|td| td.text().collect::<String>().trim().to_owned()));

        if tds.len() < 10 {
            continue;
        }

        let mut dto = RevenueDto::from(tds);
        dto.date = date;
        revenues.push(dto);
    }

    revenues
}

#[cfg(test)]
mod tests {
    use chrono::prelude::*;
    use chrono::{Local, TimeDelta};
    use std::time::Duration;

    use super::*;

    /// 最小 HTML fixture：兩筆有效資料列 + 標題列（應被跳過）。
    ///
    /// 欄位順序：代號 | 名稱 | 當月 | 上月 | 去年當月 | 當月累計 | 去年累計 | 上月增減% | 去年增減% | 前期增減%
    const FIXTURE_HTML: &str = r#"<html><body><table>
<tr><th>公司代號</th><th>公司名稱</th><th>當月營收</th><th>上月營收</th><th>去年當月營收</th>
    <th>當月累計</th><th>去年累計</th><th>上月增減%</th><th>去年增減%</th><th>前期增減%</th></tr>
<tr>
  <td>2330</td><td>台積電</td>
  <td>100,000,000</td><td>90,000,000</td><td>80,000,000</td>
  <td>500,000,000</td><td>450,000,000</td>
  <td>11.11</td><td>25.00</td><td>11.11</td>
</tr>
<tr>
  <td>2317</td><td>鴻海</td>
  <td>50,000,000</td><td>45,000,000</td><td>40,000,000</td>
  <td>200,000,000</td><td>180,000,000</td>
  <td>11.11</td><td>25.00</td><td>11.11</td>
</tr>
</table></body></html>"#;

    #[test]
    fn test_parse_revenue_html_count() {
        let result = parse_revenue_html(FIXTURE_HTML, 2026, 5);
        assert_eq!(result.len(), 2, "標題列應被跳過，只留兩筆");
    }

    #[test]
    fn test_parse_revenue_html_date_field() {
        let result = parse_revenue_html(FIXTURE_HTML, 2026, 5);
        assert!(result.iter().all(|dto| dto.date == 202605));
    }

    #[test]
    fn test_parse_revenue_html_symbol() {
        let result = parse_revenue_html(FIXTURE_HTML, 2026, 5);
        assert_eq!(result[0].stock_symbol, "2330");
        assert_eq!(result[1].stock_symbol, "2317");
    }

    #[test]
    fn test_parse_revenue_html_skips_short_rows() {
        // 不足 10 欄的資料列應被跳過
        let html = r#"<html><body><table>
<tr><td>1234</td><td>公司</td><td>100</td></tr>
</table></body></html>"#;
        let result = parse_revenue_html(html, 2026, 5);
        assert!(result.is_empty());
    }

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
                tracing::debug!("Failed to visit because: {:?}", why);
            }
            Ok(list) => {
                tracing::debug!("data:{:#?}", list);
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
