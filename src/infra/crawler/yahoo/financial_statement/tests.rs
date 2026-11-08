//! Yahoo 財務報表採集器的測試。

use rust_decimal_macros::dec;

use super::*;

/// 2xx 照常反序列化；非 2xx 與格式不符回錯誤，錯誤訊息裡的 body 是單行片段。
#[test]
fn decode_status_and_body() {
    let url = build_url("balanceSheets", "2330", ReportPeriod::Quarter);
    let ok: Vec<i32> =
        decode("balanceSheets", &url, reqwest::StatusCode::OK, "[1, 2]").expect("可解析");
    assert_eq!(ok, vec![1, 2]);

    for (status, body, want) in [
        (
            reqwest::StatusCode::BAD_GATEWAY,
            "<!DOCTYPE html>\n<html lang=\"en-us\">",
            "HTTP 502",
        ),
        (reqwest::StatusCode::OK, "{\n  \"oops\"", "Error parsing"),
    ] {
        let err = decode::<Vec<i32>>("balanceSheets", &url, status, body).expect_err("應回錯誤");
        let message = err.to_string();
        assert!(message.contains(want), "{message}");
        assert!(!message.contains('\n'), "{message}");
    }
}

#[test]
fn test_parse_fiscal_period() {
    let quarter = parse_fiscal_period("2026-06-01T00:00:00+08:00", ReportPeriod::Quarter)
        .expect("quarter date");
    assert_eq!(
        quarter,
        FiscalPeriod {
            year: 2026,
            quarter: Some(2)
        }
    );

    let cumulative =
        parse_fiscal_period("2025-12-01T00:00:00+08:00", ReportPeriod::CumulativeQuarter)
            .expect("cumulative date");
    assert_eq!(cumulative.quarter, Some(4));

    let year =
        parse_fiscal_period("2025-01-01T00:00:00+08:00", ReportPeriod::Year).expect("year date");
    assert_eq!(
        year,
        FiscalPeriod {
            year: 2025,
            quarter: None
        }
    );
}

/// 資產負債表 `period=year` 混入的逐日資料（2020-06-10）必須被擋下。
#[test]
fn test_parse_fiscal_period_rejects_mismatched_month() {
    assert!(parse_fiscal_period("2020-06-10T00:00:00+08:00", ReportPeriod::Year).is_err());
    assert!(parse_fiscal_period("2026-05-01T00:00:00+08:00", ReportPeriod::Quarter).is_err());
    assert!(parse_fiscal_period("not-a-date", ReportPeriod::Quarter).is_err());
}

#[test]
fn test_strip_exchange_suffix() {
    assert_eq!(strip_exchange_suffix("8042.TWO"), "8042");
    assert_eq!(strip_exchange_suffix("2330.TW"), "2330");
    assert_eq!(strip_exchange_suffix("2330"), "2330");
}

#[test]
fn test_deserialize_decimal() {
    #[derive(Deserialize)]
    struct Row {
        #[serde(default, deserialize_with = "deserialize_decimal")]
        value: Option<Decimal>,
    }

    let parse = |json: &str| serde_json::from_str::<Row>(json).expect("row").value;

    assert_eq!(
        parse(r#"{"value":"-52332000.00"}"#),
        Some(dec!(-52332000.00))
    );
    assert_eq!(parse(r#"{"value":"150932000"}"#), Some(dec!(150932000)));
    assert_eq!(parse(r#"{"value":1.5}"#), Some(dec!(1.5)));
    assert_eq!(parse(r#"{"value":null}"#), None);
    assert_eq!(parse(r#"{"value":""}"#), None);
    assert_eq!(parse(r#"{}"#), None);
    assert!(serde_json::from_str::<Row>(r#"{"value":"abc"}"#).is_err());
}

#[test]
fn test_build_url() {
    assert_eq!(
        build_url(
            "cashFlowStatements",
            "8042.TWO",
            ReportPeriod::CumulativeQuarter
        ),
        "https://tw.stock.yahoo.com/_td-stock/api/resource/StockServices.cashFlowStatements;limit=100;period=quarterSum;sortBy=-date;symbol=8042.TWO"
    );
}
