use anyhow::Result;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::{
    core::util::http,
    infra::cache::{TTL, TtlCacheInner},
    infra::crawler::{
        share,
        share::{DailyQuoteDto, change_range_percent},
        twse,
    },
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
    /// 回應中的資料表清單。查無資料時（休市日、臨時停市）回應沒有這個欄位，視為空清單。
    #[serde(rename = "tables", default)]
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

    #[serde(
        rename = "data",
        default,
        deserialize_with = "deserialize_nullable_cells"
    )]
    /// 表格資料列。
    pub data: Option<Vec<Vec<String>>>,

    #[serde(rename = "hints")]
    /// TWSE 附帶提示訊息。
    pub hints: Option<String>,
}

/// 反序列化表格資料列，並把 `null` 儲存格視為空字串。
///
/// TWSE 在部分日期（例如 2019-06、2019-08 的多個交易日）會在**指數**表格裡送出
/// `null` 儲存格。若照 `Vec<Vec<String>>` 直接解析，serde 會在那個 `null` 上失敗，
/// 連帶讓**整份回應**解析不出來——即使當日的每日收盤行情表格本身完全正常，
/// 該日報價也會回補失敗。這裡把 `null` 一律降級為空字串，讓解析只取決於
/// 真正需要的報價欄位。
fn deserialize_nullable_cells<'de, D>(deserializer: D) -> Result<Option<Vec<Vec<String>>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<Vec<Vec<Option<String>>>>::deserialize(deserializer)?;

    Ok(raw.map(|rows| {
        rows.into_iter()
            .map(|row| row.into_iter().map(Option::unwrap_or_default).collect())
            .collect()
    }))
}

/// TWSE `MI_INDEX` 表示「這天沒有資料」的 stat。
///
/// 臨時停市（例如 2026-10-23，不在交易所假日表上）回「很抱歉，沒有符合條件的資料!」且沒有
/// `tables`；舊版只認得「查無資料」，把它當成 API 錯誤，收盤匯總因此報錯並告警。
fn is_no_data_stat(stat: &str) -> bool {
    stat == "很抱歉，查無資料"
        || stat.contains("沒有符合條件的資料")
        || stat.contains("查詢日期大於當前日期")
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
        if is_no_data_stat(stat) {
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

        // 統計解析失敗而被拒絕的資料列數；結尾會依比例決定只警告還是整批失敗。
        let mut rejected_rows = 0usize;

        for item in rows {
            // 使用欄位映射表解析單筆資料並建立 DTO。
            // 解析失敗（欄位缺失或數值格式錯誤）時拒絕該列，不再默默補 0——
            // 半套零值資料入庫會污染均線、估價等後續計算。
            let mut dto = match DailyQuoteDto::from_with_map(item, &field_map, date) {
                Ok(dto) => dto,
                Err(why) => {
                    rejected_rows += 1;
                    // 只記錄前幾筆的完整內容，避免來源整批壞掉時 log 洗版。
                    if rejected_rows <= 5 {
                        tracing::warn!("TWSE quote row rejected: {why}, row={item:?}");
                    }
                    continue;
                }
            };

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

            // 漲跌幅以交易所的參考價（收盤 − 漲跌）為基準，與官方一致。
            dto.change_range = change_range_percent(dto.closing_price, dto.change);

            dqs.push(dto);
        }

        // 拒絕比例超過門檻（10%）代表來源格式極可能已變更：
        // 整批失敗讓呼叫端告警調查，避免大量缺漏資料悄悄入庫。
        share::ensure_rejected_rows_within_threshold("TWSE", rejected_rows, rows.len())?;
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
mod tests;
