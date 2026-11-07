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
            let field_count = item.len();
            tracing::warn!(
                "publicForm 資料列欄位不足（{} < 12），略過: {:?}",
                field_count,
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
mod tests;
