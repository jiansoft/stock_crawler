use crate::{
    core::declare::StockExchange,
    core::util,
    infra::cache::{TTL, TtlCacheInner},
    infra::crawler::{share::DailyQuoteDto, tpex},
};
use anyhow::Result;
use chrono::{Datelike, NaiveDate};
use hashbrown::HashMap;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::Deserialize;
use serde_derive::Serialize;

// QuoteResponse 上櫃公司每日收盤資訊
/*#[derive(Debug, Deserialize)]
struct QuoteResponse {
    //pub report_date: String,
    #[serde(rename = "aaData")]
    pub aa_data: Vec<Vec<String>>,
    // i_total_records: i32,
}*/

// QuoteResponse 上櫃公司每日收盤資訊
#[derive(Serialize, Deserialize, Debug)]
pub struct QuoteResponse {
    #[serde(rename = "tables")]
    pub tables: Vec<Table>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Table {
    #[serde(rename = "data")]
    pub data: Option<Vec<Vec<String>>>,
}

/// PeRatioAnalysis 上櫃股票個股本益比、殖利率、股價淨值比
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PeRatioAnalysisResponse {
    /// 證券代號
    #[serde(rename = "SecuritiesCompanyCode")]
    pub security_code: String,
    /// 本益比
    #[serde(rename = "PriceEarningRatio")]
    pub price_earning_ratio: String,
}

/// 抓取上櫃公司每日收盤資訊
pub async fn visit(date: NaiveDate) -> Result<Vec<DailyQuoteDto>> {
    let pe_ratio_url = "https://www.tpex.org.tw/openapi/v1/tpex_mainboard_peratio_analysis";
    // 本益比
    let pe_ratio_response =
        util::http::get_json::<Vec<PeRatioAnalysisResponse>>(pe_ratio_url).await?;

    let republic_date = util::datetime::gregorian_year_to_roc_year(date.year());
    //https://www.tpex.org.tw/web/stock/aftertrading/daily_close_quotes/stk_quote_result.php?l=zh-tw&_=1681801169006
    let quote_url = format!(
        "https://{}/web/stock/aftertrading/otc_quotes_no1430/stk_wn1430_result.php?l=zh-tw&d={}{}&se=EW&_={}",
        tpex::HOST,
        republic_date,
        date.format("/%m/%d"),
        date
    );

    let quote_response = util::http::get_json::<QuoteResponse>(&quote_url).await?;
    parse_quote_response(quote_response, pe_ratio_response, date).await
}

/// 解析 TPEX 每日收盤資訊與本益比資料，並將其轉換為 `DailyQuoteDto` 列表。
///
/// # 參數
/// * `quote_response` - TPEX 每日收盤資訊的回應資料。
/// * `pe_ratio_response` - TPEX 本益比分析的原始資料列表。
/// * `date` - 資料所屬的日期。
///
/// # 傳回值
/// 回傳解析後的 `DailyQuoteDto` 向量。
pub async fn parse_quote_response(
    quote_response: QuoteResponse,
    pe_ratio_response: Vec<PeRatioAnalysisResponse>,
    date: NaiveDate,
) -> Result<Vec<DailyQuoteDto>> {
    let mut pe_ratio_analysis: HashMap<String, PeRatioAnalysisResponse> =
        HashMap::with_capacity(pe_ratio_response.len());

    for item in pe_ratio_response {
        pe_ratio_analysis.insert(item.security_code.to_string(), item);
    }

    let mut dqs: Vec<DailyQuoteDto> = Vec::with_capacity(2048);
    if !quote_response.tables.is_empty()
        && let Some(tpex_dqs) = &quote_response.tables[0].data
    {
        for item in tpex_dqs {
            let mut dto = DailyQuoteDto::from_with_exchange(StockExchange::TPEx, item, date);

            if dto.closing_price.is_zero()
                && dto.highest_price.is_zero()
                && dto.lowest_price.is_zero()
                && dto.opening_price.is_zero()
            {
                continue;
            }

            let daily_quote_memory_key = format!("{}-{}", date.format("%Y%m%d"), dto.symbol);

            if TTL.daily_quote_contains_key(&daily_quote_memory_key) {
                continue;
            }

            if !dto.change.is_zero()
                && let Some(ldg) = crate::infra::cache::SHARE
                    .get_last_trading_day_quotes(&dto.symbol)
                    .await
            {
                if ldg.closing_price > Decimal::ZERO {
                    // 漲幅 = (現價 - 上一個交易日收盤價) / 上一個交易日收盤價 * 100%
                    dto.change_range =
                        (dto.closing_price - ldg.closing_price) / ldg.closing_price * dec!(100);
                } else {
                    dto.change_range = dto.change / dto.opening_price * dec!(100);
                }
            }

            if let Some(pe_ratio_analysis_response) = pe_ratio_analysis.get(&dto.symbol) {
                dto.price_earning_ratio = pe_ratio_analysis_response
                    .price_earning_ratio
                    .parse::<Decimal>()
                    .unwrap_or_default()
            }

            dqs.push(dto);
        }
    }

    Ok(dqs)
}

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeDelta, Timelike};
    use std::time::Duration;

    use crate::infra::cache::SHARE;

    use super::*;

    #[tokio::test]
    async fn test_parse_quote_response() {
        let table = Table {
            data: Some(vec![vec![
                "5483".to_string(),      // 0: 代號
                "中美晶".to_string(),    // 1: 名稱
                "100.00".to_string(),    // 2: 收盤價
                "1.50".to_string(),      // 3: 漲跌
                "98.50".to_string(),     // 4: 開盤價
                "101.00".to_string(),    // 5: 最高價
                "98.00".to_string(),     // 6: 最低價
                "10,000".to_string(),    // 7: 成交股數 (含逗號)
                "1,000,000".to_string(), // 8: 成交金額 (含逗號)
                "500".to_string(),       // 9: 成交筆數
                "100.00".to_string(),    // 10: 最後買價
                "10".to_string(),        // 11: 最後買量
                "100.50".to_string(),    // 12: 最後賣價
                "20".to_string(),        // 13: 最後賣量
            ]]),
        };

        let response = QuoteResponse {
            tables: vec![table],
        };

        let pe_ratio = vec![PeRatioAnalysisResponse {
            security_code: "5483".to_string(),
            price_earning_ratio: "12.34".to_string(),
        }];

        let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();
        let result = parse_quote_response(response, pe_ratio, date)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        let quote = &result[0];
        assert_eq!(quote.symbol, "5483");
        assert_eq!(quote.opening_price, dec!(98.50));
        assert_eq!(quote.highest_price, dec!(101.00));
        assert_eq!(quote.lowest_price, dec!(98.00));
        assert_eq!(quote.closing_price, dec!(100.00));
        assert_eq!(quote.change, dec!(1.50));
        assert_eq!(quote.trading_volume, dec!(10000));
        assert_eq!(quote.trade_value, dec!(1000000));
        assert_eq!(quote.transaction, dec!(500));
        assert_eq!(quote.price_earning_ratio, dec!(12.34));
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        SHARE.load().await;

        let mut now = Local::now();
        if now.hour() < 15 {
            now -= TimeDelta::try_days(1).unwrap();
        }
        // now -= Duration::days(3);

        tracing::debug!("開始 visit");

        match visit(now.date_naive()).await {
            Err(why) => {
                tracing::debug!("Failed to visit because: {:?}", why);
            }
            Ok(list) => {
                tracing::debug!("data:{:#?}", list);
            }
        }

        tracing::debug!("結束 visit");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
