use anyhow::Result;
use chrono::{Datelike, Local, NaiveDate, TimeDelta};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// 通知改走 core::alert 抽象介面，infra 層不直接依賴 interfaces::bot（反向耦合）。
use crate::{core::alert, core::util, core::util::map::Keyable, infra::crawler::twse};

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
struct PublicFormResponse {
    pub stat: Option<String>,
    pub date: String,
    pub title: String,
    pub fields: Vec<String>,
    pub data: Vec<Vec<String>>,
    pub notes: Vec<String>,
    pub total: i64,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
/// TWSE 公開申購頁面的單筆資料。
pub struct Public {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub stock_name: String,
    /// 發行市場
    pub market: String,
    /// 申購開始日
    pub offering_start_date: Option<NaiveDate>,
    /// 申購結束日
    pub offering_end_date: Option<NaiveDate>,
    /// 抽籤日期
    pub drawing_date: Option<NaiveDate>,
    /// 承銷價
    pub offering_price: Option<Decimal>,
    /// 撥券日期
    pub issue_date: Option<NaiveDate>,
}

impl Keyable for Public {
    fn key(&self) -> String {
        self.stock_symbol.clone()
    }

    fn key_with_prefix(&self) -> String {
        format!("Public:{}", self.key())
    }
}

impl Public {
    /// 建立一筆公開申購資料並套用預設值。
    pub fn new(stock_symbol: String, stock_name: String, market: String) -> Self {
        Self {
            stock_symbol,
            stock_name,
            market,
            offering_start_date: Default::default(),
            offering_end_date: Default::default(),
            drawing_date: Default::default(),
            offering_price: Default::default(),
            issue_date: Default::default(),
        }
    }
}

/// 抓取近期公開申購資料。
///
/// 來源為 TWSE `publicForm` JSON API。函式會將回應中的民國日期轉成西元日期，
/// 並整理為 `Public` 結構清單。
///
/// # 錯誤
///
/// 當 HTTP 請求或 JSON 解析失敗時回傳錯誤。
pub async fn visit() -> Result<Vec<Public>> {
    let now = Local::now();
    let date = now + TimeDelta::try_days(5).unwrap();
    let url = format!(
        "https://www.{host}/rwd/zh/announcement/publicForm?date={year}&response=json&_={time}",
        host = twse::HOST,
        year = date.year(),
        time = now.timestamp_millis()
    );
    let res = util::http::get_json::<PublicFormResponse>(&url).await?;
    let mut result: Vec<Public> = Vec::with_capacity(2048);
    let stat = match res.stat {
        None => {
            let to_bot_msg = "Public\\.res\\.Stat is None";
            alert::send_message(to_bot_msg).await;
            return Ok(result);
        }
        Some(stat) => stat.to_uppercase(),
    };

    if stat != "OK" {
        let to_bot_msg = "Public\\.res\\.Stat is not ok";
        alert::send_message(to_bot_msg).await;
        return Ok(result);
    }

    result.extend(map_public_items(res.data));

    Ok(result)
}

/// 將 TWSE `publicForm` API 的原始資料列整理成 [`Public`] 清單。
///
/// 這是一個「純函式」——輸入只有已反序列化的資料列，不做任何網路 I/O，
/// 可直接用組好的資料驗證（見下方測試）。民國日期（`115/07/21`）
/// 在這裡轉成西元 `NaiveDate`；承銷價無法解析（如 `-`）時為 `None`。
fn map_public_items(data: Vec<Vec<String>>) -> Vec<Public> {
    let mut result: Vec<Public> = Vec::with_capacity(data.len());

    for item in data {
        // ["序號", "抽籤日期", "證券名稱", "證券代號", "發行市場",
        //  5"申購開始日", 6"申購結束日", "承銷股數", "實際承銷股數", "承銷價(元)",
        // 10 "實際承銷價(元)", 撥券日期(上市、上櫃日期)]
        //
        // 防禦：下面直接以索引取值到 item[11]，來源若某列欄位不足，
        // 直接索引會讓整個爬蟲 panic——寧可略過該列並留下紀錄。
        if item.len() < 12 {
            tracing::warn!(
                "publicForm 資料列欄位不足（{} < 12），略過: {:?}",
                item.len(),
                item
            );
            continue;
        }

        let mut p = Public::new(item[3].clone(), item[2].clone(), item[4].clone());
        p.drawing_date = util::datetime::parse_taiwan_date(&item[1]);
        p.offering_start_date = util::datetime::parse_taiwan_date(&item[5]);
        p.offering_end_date = util::datetime::parse_taiwan_date(&item[6]);
        p.issue_date = util::datetime::parse_taiwan_date(&item[11]);
        p.offering_price = util::text::parse_decimal(&item[10], None).ok();

        result.push(p);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::cache::SHARE;
    use rust_decimal_macros::dec;

    /// 以貼近 publicForm 真實回應形狀的資料驗證整列整理流程。
    ///
    /// 涵蓋：民國日期轉西元、承銷價「-」→ None、
    /// 以及欄位不足列的防護（略過而不是 panic）。
    #[test]
    fn map_public_items_maps_rows_and_skips_short_rows() {
        let data = vec![
            // 正常列：12 欄，日期為民國格式。
            vec![
                "1".to_string(),          // 序號
                "115/07/21".to_string(),  // 抽籤日期
                "測試公司".to_string(),   // 證券名稱
                "9999".to_string(),       // 證券代號
                "上市".to_string(),       // 發行市場
                "115/07/14".to_string(),  // 申購開始日
                "115/07/16".to_string(),  // 申購結束日
                "10,000,000".to_string(), // 承銷股數
                "8,000,000".to_string(),  // 實際承銷股數
                "50.00".to_string(),      // 承銷價(元)
                "52.50".to_string(),      // 實際承銷價(元)
                "115/07/28".to_string(),  // 撥券日期
            ],
            // 承銷價尚未公布（「-」）→ offering_price 應為 None。
            vec![
                "2".to_string(),
                "尚未公布".to_string(),
                "另一家公司".to_string(),
                "8888".to_string(),
                "上櫃".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
            ],
            // 欄位不足列：直接索引會 panic，必須被防護略過。
            vec!["3".to_string(), "殘缺列".to_string()],
        ];

        let result = map_public_items(data);

        assert_eq!(result.len(), 2, "欄位不足列應被略過");

        let first = &result[0];
        assert_eq!(first.stock_symbol, "9999");
        assert_eq!(first.stock_name, "測試公司");
        assert_eq!(first.market, "上市");
        // 民國 115/07/21 → 西元 2026-07-21。
        assert_eq!(
            first.drawing_date,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 21)
        );
        assert_eq!(
            first.offering_start_date,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 14)
        );
        assert_eq!(
            first.issue_date,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 28)
        );
        // 承銷價取「實際承銷價」欄（index 10）。
        assert_eq!(first.offering_price, Some(dec!(52.50)));

        let second = &result[1];
        assert_eq!(second.stock_symbol, "8888");
        assert_eq!(second.drawing_date, None, "「尚未公布」不是日期 → None");
        assert_eq!(second.offering_price, None, "「-」無法解析 → None");
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 visit");

        match visit().await {
            Ok(list) => {
                //dbg!(&list);
                tracing::debug!("list:{list:#?}");
            }
            Err(why) => {
                tracing::debug!("Failed to visit because: {why:?}");
            }
        }

        tracing::debug!("結束 visit");
    }
}
