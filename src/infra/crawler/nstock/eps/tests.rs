//! NStock EPS crawler的測試；連外的測試標 #[ignore]。

use super::*;

/// 驗證年季字串正常解析。
#[test]
fn test_parse_year_and_quarter_parses_valid_input() {
    assert_eq!(parse_year_and_quarter("202301").unwrap(), (2023, 1));
    assert_eq!(parse_year_and_quarter("202504").unwrap(), (2025, 4));
}

/// 驗證含多位元組字元的輸入回傳錯誤而不是 panic。
///
/// 「台台」恰好是 6 bytes，舊版只檢查 len() == 6 就做 `[..4]` 切片，
/// byte 4 落在第二個字的中間會直接 panic。
#[test]
fn test_parse_year_and_quarter_rejects_multibyte_input() {
    assert!(parse_year_and_quarter("台台").is_err()); // 6 bytes，切 ..4 會跨字元
    assert!(parse_year_and_quarter("２０２３01").is_err()); // 全形數字
}

/// 驗證長度不符或含非數字的輸入回傳錯誤。
#[test]
fn test_parse_year_and_quarter_rejects_invalid_input() {
    assert!(parse_year_and_quarter("").is_err());
    assert!(parse_year_and_quarter("2023Q1").is_err()); // 含英文字母
    assert!(parse_year_and_quarter("20231").is_err()); // 只有 5 碼
}

// === JSON fixture 測試（離線、不連網路）===
//
// fixture 內容位於 testdata/eps_response.json，是手工構造的代表性樣本
// （非真實數據），涵蓋正常資料與 API 常見的髒資料（"N/A"、格式錯的年季）。
// 這是本專案「crawler fixture 測試模式」的 JSON 版示範：
// 1. serde 反序列化 fixture（同時驗證中文欄位名的 rename 對應仍正確）。
// 2. 呼叫純函式 parse_eps_response 驗證 DTO → 專案模型的轉換規則。

/// 驗證 fixture 中的正常資料被完整轉換、髒資料被逐筆丟棄。
#[test]
fn parse_eps_response_filters_dirty_rows_from_fixture() {
    // include_str! 相對於本檔案（nstock/eps/tests.rs）→ nstock/testdata/。
    const FIXTURE: &str = include_str!("../testdata/eps_response.json");

    // 第一步：反序列化。若站方欄位改名（例如「年季」改「年/季」），
    // 這裡就會失敗，等於順帶測到 serde rename 的對應。
    let res: EpsResponse = serde_json::from_str(FIXTURE).expect("fixture 應可反序列化");

    // 第二步：轉換。fixture 有 4 筆季度（1 筆年季格式錯、1 筆 EPS 為 N/A）
    // 與 3 筆年度（1 筆年度為 N/A）——髒資料應被逐筆丟棄，不影響其他筆。
    let eps = parse_eps_response(res, "2330");

    assert_eq!(eps.quarters.len(), 2, "quarters: {:#?}", eps.quarters);
    assert_eq!(eps.quarters[0].year, 2023);
    assert_eq!(eps.quarters[0].quarter, Quarter::Q1);
    assert_eq!(eps.quarters[0].eps, rust_decimal_macros::dec!(7.98));
    assert_eq!(
        eps.quarters[1].cumulative_eps,
        rust_decimal_macros::dec!(14.99)
    );

    assert_eq!(eps.years.len(), 2, "years: {:#?}", eps.years);
    assert_eq!(eps.years[0].year, 2023);
    assert_eq!(eps.years[0].eps, rust_decimal_macros::dec!(32.34));
    assert_eq!(eps.years[1].gross_profit, rust_decimal_macros::dec!(59.56));

    // Keyable 的鍵格式順帶驗證（快取層以此當 Redis/記憶體鍵）。
    assert_eq!(eps.quarters[0].key(), "2330-2023-Q1");
}

/// 沒有資料的股票回 `{"data":[{}]}`（2026-10-29 的 3718）：視為沒有季度與年度資料，不是格式錯誤。
#[test]
fn empty_data_object_means_no_eps() {
    for body in [r#"{"data":[{}]}"#, r#"{"data":[]}"#] {
        let res: EpsResponse = serde_json::from_str(body).expect("空資料應可反序列化");
        let eps = parse_eps_response(res, "3718");
        assert!(eps.quarters.is_empty() && eps.years.is_empty(), "{body}");
    }
    // 欄位型別不符仍是錯誤（站方改版時要看得到）。
    assert!(serde_json::from_str::<EpsResponse>(r#"{"data":[{"季度EPS":"x"}]}"#).is_err());
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_visit() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 visit");

    match visit("2330").await {
        Ok(e) => {
            dbg!(&e);
            tracing::debug!("nstock : {:#?}", e);
        }
        Err(why) => {
            tracing::debug!("Failed to visit because {:?}", why);
        }
    }

    tracing::debug!("結束 visit");
}
