use crate::{
    crawler::bank_of_taiwan,
    util,
    util::{http, text}
};
use anyhow::anyhow;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use scraper::{Html, Selector};

/// 基金資訊結構體，包含基金的基本資料與相關數據
#[derive(Debug)]
struct FundInfo {
    /// 基金名稱，例如："高盛邊境市場債券基金X股"
    pub fund_name: String,

    /// 除息日（Ex-Dividend Date），格式為西元年 `YYYY-MM-DD`
    /// - 民國 `114/02/11` 會轉換為 `2025-02-11`
    pub ex_dividend_date: NaiveDate,

    /// 單位價格（Unit Price），使用 `Decimal` 存儲以確保精度
    /// - 例如："2,429.1300" 會轉換為 `2429.1300`
    pub unit_price: Decimal,

    /// 記錄日（Record Date），格式為西元年 `YYYY-MM-DD`
    /// - 民國 `114/02/04` 會轉換為 `2025-02-04`
    pub record_date: NaiveDate,

    /// 配息率（Dividend Yield），使用 `Decimal` 存儲以確保精度
    /// - 例如："27.7" 會轉換為 `27.7`
    pub dividend_yield: Decimal,

    /// 幣別（Currency），例如："南非幣"（ZAR）、"美元"（USD）
    pub currency: String,

    /// 配息頻率（Payout Frequency），例如："月配息"（每月配息）、"年配息"（每年配息）
    pub payout_frequency: String,

    /// 基金詳細資訊的 URL
    pub fund_url: String,
}

impl FundInfo {
    fn from_tds(tds: Vec<String>) -> anyhow::Result<Self> {
        if tds.len() < 9 {
            return Err(anyhow!("Insufficient data in tds array"));
        }

        Ok(Self {
            fund_name: extract_fund_name(&tds[0]),
            ex_dividend_date: util::datetime::parse_taiwan_date(&tds[1])
                .ok_or_else(|| anyhow!("Failed to parse ROC date: {}", tds[1]))?,
            unit_price: text::parse_decimal(&tds[2], Some(vec![',']))?,
            record_date: util::datetime::parse_taiwan_date(&tds[3])
                .ok_or_else(|| anyhow!("Failed to parse ROC date: {}", tds[3]))?,
            dividend_yield: text::parse_decimal(&tds[4], Some(vec![',']))?,
            currency: tds[5].clone(),
            payout_frequency: tds[6].clone(),
            fund_url: tds[8].clone(),
        })
    }
}

fn extract_fund_name(full_name: &str) -> String {
    let name_without_prefix = full_name
        .split_whitespace()
        .skip(1) // 跳過第一個單詞（基金代碼）
        .collect::<Vec<&str>>() // 轉為 Vec
        .join(" "); // 重新合併成字串

    if let Some(pos) = name_without_prefix.find('(') {
        name_without_prefix[..pos].trim().to_string()
    } else {
        name_without_prefix // 如果沒有括號，則回傳完整名稱
    }
}
pub async fn visit() -> anyhow::Result<()> {
    let url = format!(
        "https://{}/w/FundDivYieldorderby.djhtm",
        bank_of_taiwan::HOST
    );
    let text = http::get(&url, None).await?;
    let document = Html::parse_document(&text);
    let selector = Selector::parse("#oMainTable > tbody > tr:nth-child(n+3)")
        .map_err(|why| anyhow!("Failed to Selector::parse because: {:?}", why))?;
    let td_selector = Selector::parse("td").expect("Failed to parse td selector");
    let link_selector = Selector::parse("a").expect("Failed to parse a selector");
    for node in document.select(&selector) {
        let mut tds: Vec<String> = node
            .select(&td_selector)
            .map(|td| td.text().collect::<String>().trim().to_string())
            .collect();

        // 提取 <a> 標籤中的 href 屬性
        let fund_url = node
            .select(&link_selector)
            .next()
            .and_then(|a_node| a_node.value().attr("href"))
            .map_or(String::from(""), String::from);
        tds.push(fund_url);

        let fund_info = FundInfo::from_tds(tds)?;
        println!("{:#?}", fund_info);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::crawler::bank_of_taiwan;
    use crate::logging;

    #[tokio::test]
    async fn test_visit() {
        match bank_of_taiwan::fund::fund_list::visit().await {
            Ok(ip) => {
                dbg!(ip);
            }
            Err(why) => {
                logging::error_file_async(format!("Failed to get because {:?}", why));
            }
        }
    }
}
