//! BigGo 法說會的測試。

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
    let calls = parse_list(data(include_str!(
        "../testdata/earnings_call_list_2330.json"
    )))
    .unwrap();
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
    let raw: RawDetail = data(include_str!("../testdata/earnings_call_detail_2330.json")).unwrap();
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
