//! # BigGo 法說會
//!
//! BigGo 財經整理的台股法說會：每場有逐字稿與 AI 整理的摘要、營運重點、展望與 Q&A 重點。
//! 只取通知需要的文字欄位，**不保存逐字稿**；這些整理內容是 BigGo 的著作，只做個人通知用。
//!
//! | 函式 | endpoint |
//! |------|----------|
//! | [`list`] | `/stock/{代號}.TW/earnings-calls/list`（由新到舊） |
//! | [`detail`] | `/stock/earnings-calls/detail?call_id=…&stock_id=…` |
//!
//! `call_id` 形如 `TW_2330.TW_2026-07-16`；同一天兩場時 BigGo 會給不同的 `call_id`。
//! 摘要等欄位是法說會結束後才陸續產生的，剛舉行的場次可能還是空字串。

use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use serde::Deserialize;

use super::{api_symbol, fetch};

/// BigGo 法說會頁面的網址前綴。
const PAGE_BASE: &str = "https://finance.biggo.com.tw/quote";

/// 法說會清單中的一場。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EarningsCall {
    /// BigGo 的場次識別（例如 `TW_2330.TW_2026-07-16`）。
    pub call_id: String,
    /// 舉行日期。
    pub date: NaiveDate,
    /// 標題（例如 `台積電 2026-07-16 法說會`）。
    pub title: String,
    /// 是否已有逐字稿；沒有逐字稿的場次也不會有 AI 整理內容。
    pub has_transcript: bool,
}

/// 單場法說會的 AI 整理內容。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EarningsCallDetail {
    /// BigGo 的場次識別。
    pub call_id: String,
    /// 舉行日期。
    pub date: Option<NaiveDate>,
    /// 標題。
    pub title: String,
    /// 摘要；尚未產生時為空字串。
    pub summary: String,
    /// 營運重點。
    pub operations: String,
    /// 展望。
    pub outlook: String,
    /// Q&A 重點。
    pub qa_highlights: String,
}

impl EarningsCallDetail {
    /// AI 整理內容是否已產生（至少有摘要）。
    pub fn is_ready(&self) -> bool {
        !self.summary.trim().is_empty()
    }
}

/// BigGo 法說會頁面網址。
pub fn page_url(stock_symbol: &str, call_id: &str) -> String {
    format!(
        "{PAGE_BASE}/{}/earnings-call/{call_id}",
        api_symbol(stock_symbol)
    )
}

/// 抓取單一股票的法說會清單（由新到舊）。
///
/// # Errors
///
/// 請求失敗、回應格式不符或日期無法解析時回傳錯誤。
pub async fn list(stock_symbol: &str) -> Result<Vec<EarningsCall>> {
    let path = format!("/stock/{}/earnings-calls/list", api_symbol(stock_symbol));
    let data: Option<RawList> = fetch(&path, &[]).await?;
    parse_list(data)
}

/// 抓取單場法說會的 AI 整理內容。
///
/// # Errors
///
/// 請求失敗、回應格式不符或 BigGo 沒有這一場時回傳錯誤。
pub async fn detail(stock_symbol: &str, call_id: &str) -> Result<EarningsCallDetail> {
    let symbol = api_symbol(stock_symbol);
    let data: Option<RawDetail> = fetch(
        "/stock/earnings-calls/detail",
        &[("call_id", call_id), ("stock_id", &symbol)],
    )
    .await?;
    let data = data.ok_or_else(|| anyhow!("BigGo has no earnings call {call_id}"))?;
    Ok(parse_detail(data))
}

/// 清單 endpoint 的 `data`。
#[derive(Debug, Deserialize)]
struct RawList {
    #[serde(default)]
    list: Vec<RawListItem>,
}

/// 清單中的一場（只取需要的欄位）。
#[derive(Debug, Deserialize)]
struct RawListItem {
    call_id: String,
    date: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    has_transcript: bool,
}

/// 單場 endpoint 的 `data`（只取需要的欄位；逐字稿不取）。
#[derive(Debug, Deserialize)]
struct RawDetail {
    call_id: String,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    operations: Option<String>,
    #[serde(default)]
    outlook: Option<String>,
    #[serde(default)]
    qa_highlights: Option<String>,
}

/// 解析清單；查無資料（`null`）時回空陣列，日期無法解析時整批失敗。
fn parse_list(data: Option<RawList>) -> Result<Vec<EarningsCall>> {
    data.map(|data| data.list)
        .unwrap_or_default()
        .into_iter()
        .map(|item| {
            let date = NaiveDate::parse_from_str(&item.date, "%Y-%m-%d").with_context(|| {
                format!(
                    "Unexpected BigGo earnings call date {:?} for {}",
                    item.date, item.call_id
                )
            })?;
            Ok(EarningsCall {
                call_id: item.call_id,
                date,
                title: item.title,
                has_transcript: item.has_transcript,
            })
        })
        .collect()
}

/// 解析單場內容；缺少的文字欄位視為空字串。
fn parse_detail(data: RawDetail) -> EarningsCallDetail {
    EarningsCallDetail {
        call_id: data.call_id,
        date: data
            .date
            .and_then(|date| NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok()),
        title: data.title,
        summary: data.summary.unwrap_or_default(),
        operations: data.operations.unwrap_or_default(),
        outlook: data.outlook.unwrap_or_default(),
        qa_highlights: data.qa_highlights.unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;

    use super::*;
    use crate::infra::crawler::biggo::Envelope;

    fn data<T: DeserializeOwned>(json: &str) -> Option<T> {
        let envelope: Envelope<T> = serde_json::from_str(json).expect("fixture should parse");
        assert!(envelope.result);
        envelope.data
    }

    #[test]
    fn parse_list_reads_calls_newest_first() {
        let calls =
            parse_list(data(include_str!("testdata/earnings_call_list_2330.json"))).unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].call_id, "TW_2330.TW_2026-07-16");
        assert_eq!(calls[0].date, NaiveDate::from_ymd_opt(2026, 7, 16).unwrap());
        assert!(calls[0].has_transcript);
        assert!(calls[0].title.contains("法說會"));
        assert!(calls[0].date > calls[1].date);
    }

    #[test]
    fn parse_list_handles_no_data_and_rejects_bad_dates() {
        assert!(parse_list(None).unwrap().is_empty());
        let bad: RawList = serde_json::from_value(serde_json::json!({
            "list": [{"call_id": "TW_2330.TW_x", "date": "2026/07/16"}]
        }))
        .unwrap();
        assert!(parse_list(Some(bad)).is_err());
    }

    /// 2330 2026-07-16：摘要、展望、Q&A 都有內容；逐字稿不取。
    #[test]
    fn parse_detail_reads_ai_sections() {
        let raw: RawDetail = data(include_str!("testdata/earnings_call_detail_2330.json")).unwrap();
        let detail = parse_detail(raw);
        assert_eq!(detail.call_id, "TW_2330.TW_2026-07-16");
        assert_eq!(detail.date, NaiveDate::from_ymd_opt(2026, 7, 16));
        assert!(detail.is_ready());
        assert!(detail.summary.contains("台積電"));
        assert!(!detail.outlook.is_empty());
        assert!(detail.qa_highlights.contains("Q:"));
    }

    #[test]
    fn detail_is_not_ready_without_summary() {
        let raw: RawDetail = serde_json::from_value(serde_json::json!({
            "call_id": "TW_9999.TW_2026-10-05", "summary": "  ", "outlook": null
        }))
        .unwrap();
        let detail = parse_detail(raw);
        assert!(!detail.is_ready());
        assert_eq!(detail.outlook, "");
        assert_eq!(detail.date, None);
    }

    #[test]
    fn page_url_points_to_the_call() {
        assert_eq!(
            page_url("2330", "TW_2330.TW_2026-07-16"),
            "https://finance.biggo.com.tw/quote/2330.TW/earnings-call/TW_2330.TW_2026-07-16"
        );
    }
}
