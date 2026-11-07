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
mod tests;
