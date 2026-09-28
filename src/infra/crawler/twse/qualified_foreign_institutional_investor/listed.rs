use anyhow::Result;
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::{
    core::util::{convert::FromValue, http},
    infra::crawler::share::QfiiDto,
    infra::crawler::twse,
};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
/// TWSE 外資及陸資持股統計 API 回應。
///
/// 查詢休市日（例如平日的國定假日）時 TWSE 只回
/// `{"stat":"查詢日期大於可查詢最大日期，請重新查詢!","total":0}`，
/// 其餘欄位都不存在；因此除了 `stat` 以外的欄位一律可缺省，
/// 讓流程能走到下方的 `stat` 判斷，而不是在反序列化階段就失敗。
#[serde(default)]
pub struct QFIIResponse {
    /// 回應狀態字串。
    pub stat: Option<String>,
    /// 查詢日期。
    pub date: Option<String>,
    #[serde(rename = "selectType")]
    /// 查詢條件類型。
    pub select_type: String,
    /// 回應標題。
    pub title: Option<String>,
    /// 額外提示文字。
    pub hints: Option<String>,
    /// 欄位名稱清單。
    pub fields: Vec<String>,
    /// 原始資料列。
    pub data: Vec<Vec<serde_json::Value>>,
    /// 總筆數。
    pub total: i32,
}

/// 取得上市股票外資及陸資投資持股統計
pub async fn visit(date_time: DateTime<FixedOffset>) -> Result<Vec<QfiiDto>> {
    let url = format!(
        "https://www.{}/rwd/zh/fund/MI_QFIIS?date={}&selectType=ALLBUT0999&response=json&_={}",
        twse::HOST,
        date_time.format("%Y%m%d"),
        date_time.timestamp_millis()
    );

    let listed = http::get_json::<QFIIResponse>(&url).await?;
    let mut result = Vec::with_capacity(1024);
    let stat = match listed.stat {
        None => {
            tracing::warn!(
                "{}",
                "取得外資及陸資投資持股統計 Finish taiex.Stat is None".to_string(),
            );
            return Ok(result);
        }
        Some(stat) => stat.to_uppercase(),
    };

    if stat != "OK" {
        // 休市日也會走到這裡（stat 為「查詢日期大於可查詢最大日期」），不是錯誤。
        tracing::warn!(
            "取得外資及陸資投資持股統計 {} 無資料，略過：{}",
            date_time.format("%Y-%m-%d"),
            stat
        );
        return Ok(result);
    }

    result.extend(map_qfii_rows(listed.data));

    Ok(result)
}

/// 將 TWSE `MI_QFIIS` API 的原始資料列整理成 [`QfiiDto`] 清單。
///
/// 這是一個「純函式」——輸入只有已反序列化的 JSON 資料列，不做任何網路 I/O，
/// 可直接用組好的資料驗證（見下方測試）。
///
/// # 欄位對應（每列 12 欄）
/// - `item[0]`＝股票代號、`item[3]`＝發行股數、
///   `item[5]`＝外資及陸資持有股數、`item[7]`＝持股比率(%)。
/// - 數值欄位是含千分位逗號的字串，逗號在轉型時自動去除。
/// - 欄位數不是 12 的列（表尾統計、備註）直接略過。
fn map_qfii_rows(data: Vec<Vec<serde_json::Value>>) -> Vec<QfiiDto> {
    let mut result = Vec::with_capacity(data.len());

    for item in data {
        if item.len() != 12 {
            continue;
        }
        let stock_symbol = item[0].get_string(None);
        let issued_share = item[3].get_i64(None);
        let shares_held = item[5].get_i64(None);
        let share_holding_percentage = item[7].get_decimal(None);

        result.push(QfiiDto {
            stock_symbol,
            issued_share,
            shares_held,
            share_holding_percentage,
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use std::result::Result::Ok;

    use crate::infra::cache::SHARE;

    use super::*;
    use rust_decimal_macros::dec;
    use serde_json::json;

    /// 休市日的回應只有 `stat` 與 `total`，必須能反序列化，才能交給 `stat` 判斷略過。
    /// 2026-09-25（中秋節）、2026-09-28（教師節）排程曾因此在解析 JSON 時失敗並發出告警。
    #[test]
    fn response_on_market_holiday_deserializes_without_data() {
        let body = r#"{"stat":"查詢日期大於可查詢最大日期，請重新查詢!","total":0}"#;

        let response: QFIIResponse = serde_json::from_str(body).expect("休市日回應應可解析");

        assert_eq!(
            response.stat.as_deref(),
            Some("查詢日期大於可查詢最大日期，請重新查詢!")
        );
        assert!(response.data.is_empty());
        assert!(response.select_type.is_empty());
    }

    /// 以貼近 MI_QFIIS 真實回應形狀的資料驗證資料列整理流程。
    ///
    /// TWSE 的數值欄位都是「含千分位逗號的字串」，
    /// 整理時要正確去逗號；欄位數不是 12 的列（表尾統計）要略過。
    #[test]
    fn map_qfii_rows_maps_rows_and_skips_malformed() {
        let data: Vec<Vec<serde_json::Value>> = vec![
            // 正常列：12 欄。[0]=代號、[3]=發行股數、[5]=外資持股、[7]=持股比率。
            vec![
                json!("2330"),
                json!("台積電"),
                json!("100.00"),
                json!("25,932,070,990"),
                json!("0"),
                json!("19,381,867,412"),
                json!("0"),
                json!("74.74"),
                json!("25.26"),
                json!("100.00"),
                json!("0.00"),
                json!(""),
            ],
            // 欄位數不足的統計列 → 略過。
            vec![json!("合計"), json!("123,456")],
        ];

        let result = map_qfii_rows(data);

        assert_eq!(result.len(), 1, "欄位數不是 12 的列應被略過");
        assert_eq!(result[0].stock_symbol, "2330");
        assert_eq!(result[0].issued_share, 25_932_070_990);
        assert_eq!(result[0].shares_held, 19_381_867_412);
        assert_eq!(result[0].share_holding_percentage, dec!(74.74));
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 visit");
        //let date =  DateTime::parse_from_str("2023-09-14 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let datetime_local: DateTime<FixedOffset> =
            match DateTime::parse_from_str("2023-09-15 12:00:00 +0800", "%Y-%m-%d %H:%M:%S %z") {
                Ok(dt) => dt,
                Err(why) => {
                    tracing::debug!("error:{:#?}", why);
                    return;
                }
            };
        match visit(datetime_local).await {
            Err(why) => {
                tracing::debug!("Failed to visit because: {:?}", why);
            }
            Ok(qfiis) => {
                tracing::debug!("qfiis:{:#?}", qfiis);
            }
        }
        tracing::debug!("結束 visit");
    }
}
