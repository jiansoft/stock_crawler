use anyhow::Result;
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

use crate::{
    core::util::http,
    infra::cache::{TTL, TtlCacheInner},
    infra::crawler::{share::DailyQuoteDto, twse},
};

/*#[derive(Serialize, Deserialize, Debug)]
struct ListedResponse {
    pub stat: Option<String>,
    pub data9: Option<Vec<Vec<String>>>,
}*/

#[derive(Serialize, Deserialize, Debug)]
/// TWSE `MI_INDEX` API 的回應主體。
///
/// 這個 crawler 只使用 `tables` 中的「上市每日收盤行情」表格資料。
pub struct ListedResponse {
    /// API 回應狀態。
    pub stat: Option<String>,
    #[serde(rename = "tables")]
    /// 回應中的資料表清單。
    pub tables: Vec<Table>,
}

#[derive(Serialize, Deserialize, Debug)]
/// TWSE `MI_INDEX` 回應中的單一表格區塊。
pub struct Table {
    #[serde(rename = "title")]
    /// 表格標題。
    pub title: Option<String>,

    #[serde(rename = "fields")]
    /// 欄位名稱清單。
    pub fields: Option<Vec<String>>,

    #[serde(rename = "data")]
    /// 表格資料列。
    pub data: Option<Vec<Vec<String>>>,

    #[serde(rename = "hints")]
    /// TWSE 附帶提示訊息。
    pub hints: Option<String>,
}

/// 判斷是否為 TWSE 上市每日收盤行情表格。
///
/// 因 TWSE `tables` 順序可能調整，這裡改用欄位名稱特徵比對，
/// 避免依賴固定索引位置。
fn is_twse_quote_table(table: &Table) -> bool {
    let Some(fields) = &table.fields else {
        return false;
    };

    if fields.len() < 16 {
        return false;
    }

    let required = ["證券代號", "成交股數", "開盤價", "收盤價", "漲跌價差"];
    required
        .iter()
        .all(|key| fields.iter().any(|f| f.contains(key)))
}

/// 抓取上市公司每日收盤資訊
///
/// 資料來源：`/exchangeReport/MI_INDEX?type=ALLBUT0999`
///
/// 函式會：
/// 1. 下載 TWSE JSON 並動態定位目標表格。
/// 2. 轉換為 `DailyQuote`，過濾無效價格與記憶體去重資料。
/// 3. 盡可能計算 `change_range` 後回傳當日清單。
///
/// 若 API 可連線但找不到目標表格，會記錄 warning 並回傳空陣列。
pub async fn visit(date: NaiveDate) -> Result<Vec<DailyQuoteDto>> {
    let date_str = date.format("%Y%m%d").to_string();
    let now_ts = Local::now().timestamp_millis();
    let url = format!(
        "https://www.{}/exchangeReport/MI_INDEX?response=json&date={}&type=ALLBUT0999&_={}",
        twse::HOST,
        date_str,
        now_ts
    );

    //let headers = build_headers().await;
    let data = http::get_json::<ListedResponse>(&url).await?;
    parse_listed_response(data, date).await
}

/// 解析 TWSE MI_INDEX API 的回應資料，並將其轉換為 `DailyQuoteDto` 列表。
///
/// # 參數
/// * `data` - 從 TWSE 取得的原始 `ListedResponse` 資料。
/// * `date` - 資料所屬的日期。
///
/// # 傳回值
/// 回傳解析後的 `DailyQuoteDto` 向量，若查無資料則回傳空向量。
pub async fn parse_listed_response(
    data: ListedResponse,
    date: NaiveDate,
) -> Result<Vec<DailyQuoteDto>> {
    // 檢查 API 回應狀態，如查無資料則直接返回空陣列
    if let Some(stat) = &data.stat {
        if stat == "很抱歉，查無資料" || stat.contains("查詢日期大於當前日期") {
            return Ok(vec![]);
        }
        if stat != "OK" {
            anyhow::bail!("TWSE MI_INDEX API Error: {}", stat);
        }
    }

    let mut dqs = Vec::with_capacity(2048);
    // 使用特徵比對來尋找上市每日收盤行情的目標表格
    let target_table = data.tables.iter().find(|t| is_twse_quote_table(t));

    if let (Some(table), Some(rows)) = (target_table, target_table.and_then(|t| t.data.as_ref())) {
        // 建立欄位名稱與索引的映射表，避免硬編碼索引位置
        let mut field_map = std::collections::HashMap::new();
        if let Some(fields) = &table.fields {
            for (i, field) in fields.iter().enumerate() {
                field_map.insert(field.as_str(), i);
            }
        }

        for item in rows {
            // 使用欄位映射表解析單筆資料並建立 DTO
            let mut dto = DailyQuoteDto::from_with_map(item, &field_map, date);

            // 過濾掉開高低收皆為零的無效資料（例如暫停交易的股票）
            if dto.closing_price.is_zero()
                && dto.highest_price.is_zero()
                && dto.lowest_price.is_zero()
                && dto.opening_price.is_zero()
            {
                continue;
            }

            // 檢查記憶體中的 TTL 快取，避免重複處理相同日期的資料
            let daily_quote_memory_key = format!("{}-{}", date.format("%Y%m%d"), dto.symbol);

            if TTL.daily_quote_contains_key(&daily_quote_memory_key) {
                continue;
            }

            // 如果有價格變動且能取得前一交易日的收盤價，則計算漲跌幅
            if !dto.change.is_zero()
                && let Some(ldg) = crate::infra::cache::SHARE
                    .get_last_trading_day_quotes(&dto.symbol)
                    .await
            {
                if ldg.closing_price > Decimal::ZERO {
                    // 漲幅 = (現價 - 上一個交易日收盤價) / 上一個交易日收盤價 * 100%
                    dto.change_range =
                        (dto.closing_price - ldg.closing_price) / ldg.closing_price * dec!(100);
                } else if dto.opening_price > Decimal::ZERO {
                    dto.change_range = dto.change / dto.opening_price * dec!(100);
                } else {
                    dto.change_range = Decimal::ZERO;
                }
            }

            dqs.push(dto);
        }
    } else {
        // 若找不到符合欄位特徵的表格，記錄警告日誌
        tracing::warn!(
            "TWSE MI_INDEX quote table not found for date={}, stat={:?}, tables={}",
            date,
            data.stat,
            data.tables.len()
        );
    }
    Ok(dqs)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, Timelike};
    use std::time::Duration;

    use crate::infra::cache::SHARE;

    use super::*;

    #[test]
    fn test_is_twse_quote_table() {
        let table = Table {
            title: Some("每日收盤行情(全部(不含權證、牛熊證))".to_string()),
            fields: Some(vec![
                "證券代號".to_string(),
                "證券名稱".to_string(),
                "成交股數".to_string(),
                "成交筆數".to_string(),
                "成交金額".to_string(),
                "開盤價".to_string(),
                "最高價".to_string(),
                "最低價".to_string(),
                "收盤價".to_string(),
                "漲跌(+/-)".to_string(),
                "漲跌價差".to_string(),
                "最後揭示買價".to_string(),
                "最後揭示買量".to_string(),
                "最後揭示賣價".to_string(),
                "最後揭示賣量".to_string(),
                "本益比".to_string(),
            ]),
            data: None,
            hints: None,
        };

        assert!(is_twse_quote_table(&table));
    }

    #[tokio::test]
    async fn test_parse_listed_response() {
        let table = Table {
            title: Some("每日收盤行情(全部(不含權證、牛熊證))".to_string()),
            fields: Some(vec![
                "證券代號".to_string(),
                "證券名稱".to_string(),
                "成交股數".to_string(),
                "成交筆數".to_string(),
                "成交金額".to_string(),
                "開盤價".to_string(),
                "最高價".to_string(),
                "最低價".to_string(),
                "收盤價".to_string(),
                "漲跌(+/-)".to_string(),
                "漲跌價差".to_string(),
                "最後揭示買價".to_string(),
                "最後揭示買量".to_string(),
                "最後揭示賣價".to_string(),
                "最後揭示賣量".to_string(),
                "本益比".to_string(),
            ]),
            data: Some(vec![vec![
                "2330".to_string(),      // 證券代號
                "台積電".to_string(),    // 證券名稱
                "10,000".to_string(),    // 成交股數 (含逗號)
                "100".to_string(),       // 成交筆數
                "5,000,000".to_string(), // 成交金額 (含逗號)
                "500.00".to_string(),    // 開盤價
                "505.00".to_string(),    // 最高價
                "499.00".to_string(),    // 最低價
                "502.00".to_string(),    // 收盤價
                "+".to_string(),         // 漲跌(+/-)
                "2.00".to_string(),      // 漲跌價差
                "502.00".to_string(),
                "10".to_string(),
                "503.00".to_string(),
                "20".to_string(),
                "15.5".to_string(),
            ]]),
            hints: None,
        };

        let response = ListedResponse {
            stat: Some("OK".to_string()),
            tables: vec![table],
        };

        let date = NaiveDate::from_ymd_opt(2026, 6, 13).unwrap();
        let result = parse_listed_response(response, date).await.unwrap();

        assert_eq!(result.len(), 1);
        let quote = &result[0];
        assert_eq!(quote.symbol, "2330");
        assert_eq!(quote.opening_price, dec!(500.00));
        assert_eq!(quote.highest_price, dec!(505.00));
        assert_eq!(quote.lowest_price, dec!(499.00));
        assert_eq!(quote.closing_price, dec!(502.00));
        assert_eq!(quote.change, dec!(2.00));
        assert_eq!(quote.trading_volume, dec!(10000));
        assert_eq!(quote.trade_value, dec!(5000000));
        assert_eq!(quote.transaction, dec!(100));
        assert_eq!(quote.price_earning_ratio, dec!(15.5));
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        SHARE.load().await;

        let mut now = Local::now();
        if now.hour() < 15 {
            now -= TimeDelta::try_days(1).unwrap();
        }
        //now -= Duration::days(3);

        tracing::debug!("開始 visit");

        match visit(now.date_naive()).await {
            Err(why) => {
                tracing::debug!("Failed to visit because: {:?}", why);
            }
            Ok(list) => {
                tracing::debug!("data count: {}, detail:{:#?}", list.len(), list);
            }
        }

        tracing::debug!("結束 visit");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
