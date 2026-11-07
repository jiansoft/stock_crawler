//! Yahoo 個股基本面採集器的測試；連外的測試標 #[ignore]。

use super::*;
use rust_decimal_macros::dec;

/// 無有效資料略過 1 天、404 略過 30 天，其他錯誤不略過。
#[test]
fn skip_cache_ttl_seconds_by_error_kind() {
    let no_data = anyhow::Error::new(NoValidProfileDataError {
        stock_symbol: "7777".to_string(),
        url: "https://example".to_string(),
    })
    .context("wrapped");
    assert_eq!(
        skip_cache_ttl_seconds(&no_data),
        Some(NO_VALID_DATA_CACHE_TTL_SECONDS)
    );

    let not_found = anyhow::Error::new(YahooPageNotFoundError {
        stock_symbol: "4712".to_string(),
        url: "https://tw.stock.yahoo.com/quote/4712/profile".to_string(),
    })
    .context("wrapped");
    assert_eq!(
        skip_cache_ttl_seconds(&not_found),
        Some(PAGE_NOT_FOUND_CACHE_TTL_SECONDS)
    );

    let parse_failure = anyhow::anyhow!("Failed to find profile section for 2330");
    assert_eq!(skip_cache_ttl_seconds(&parse_failure), None);
}

/// 4712 已下市，Yahoo 個股頁回 404，必須辨識為頁面不存在而非解析失敗。
#[tokio::test]
#[ignore]
async fn visit_reports_page_not_found_for_delisted_stock() {
    dotenvy::dotenv().ok();
    let err = visit("4712").await.expect_err("4712 should be 404");
    assert!(is_page_not_found_error(&err), "{err:#}");
}

/// 以貼近真實 profile 頁形狀的 fixture 驗證整頁解析流程。
///
/// 涵蓋三個層級的定位：Proxy 容器下的第三個 section、
/// `div.D(f)` 的年度季度文字、CSS Grid 六格比率與獨立層級的 EPS。
/// Yahoo 改版時只要用瀏覽器存一份新回應更新 fixture，
/// 測試立即反映哪一段 selector 需要調整。
#[test]
fn parse_profile_html_parses_fixture() {
    // include_str! 的路徑相對於本檔案（yahoo/profile/tests.rs）→ yahoo/testdata/。
    const FIXTURE: &str = include_str!("../testdata/profile_page.html");

    let profile =
        parse_profile_html("2330", "https://example.test/quote/2330/profile", FIXTURE).unwrap();

    // 年度與季度來自「2025 Q3」文字：前 4 碼是年份，正則抓出季度。
    assert_eq!(profile.year, 2025);
    assert_eq!(profile.quarter, "Q3");
    assert_eq!(profile.stock_symbol, "2330");

    // Grid 第 1～6 格：毛利率、ROA、營益率、ROE、稅前淨利率、每股淨值。
    assert_eq!(profile.gross_profit, dec!(58.6));
    assert_eq!(profile.return_on_assets, dec!(12.9));
    assert_eq!(profile.operating_profit_margin, dec!(48.1));
    assert_eq!(profile.return_on_equity, dec!(26.3));
    assert_eq!(profile.pre_tax_income, dec!(53.2));
    assert_eq!(profile.net_asset_value_per_share, dec!(158.9));

    // EPS 位於另一個 grid 的第 3 格（層級不同，獨立解析）。
    assert_eq!(profile.earnings_per_share, dec!(15.36));
}

/// 頁面有 profile 區塊、但年份與 EPS 都解析不到時，
/// 應回傳 NoValidProfileDataError（可被 is_no_valid_data_error 辨識），
/// 讓 backfill 流程把它當「暫時無資料」降級處理，而不是一般錯誤。
#[test]
fn parse_profile_html_reports_no_valid_data_for_empty_section() {
    let html = r#"
        <div id="main-2-QuoteProfile-Proxy"><div>
            <section>1</section>
            <section>2</section>
            <section><h2>獲利能力</h2><div class="D(f)">尚無資料</div></section>
        </div></div>
    "#;

    let error =
        parse_profile_html("7811", "https://example.test/quote/7811/profile", html).unwrap_err();
    assert!(is_no_valid_data_error(&error));
}

/// 整頁缺少 profile 區塊（載到錯誤頁）時，回傳的是一般解析錯誤，
/// 不應被誤判為「無有效資料」的暫時性跳過。
#[test]
fn parse_profile_html_rejects_page_without_profile_section() {
    let error = parse_profile_html(
        "2330",
        "https://example.test/quote/2330/profile",
        "<html><body><h1>404</h1></body></html>",
    )
    .unwrap_err();

    assert!(!is_no_valid_data_error(&error));
    assert!(error.to_string().contains("profile section"));
}

#[test]
fn test_is_no_valid_data_error() {
    let err: anyhow::Error = NoValidProfileDataError {
        stock_symbol: "7811".to_string(),
        url: "https://tw.stock.yahoo.com/quote/7811.TWO/profile".to_string(),
    }
    .into();

    assert!(is_no_valid_data_error(&err));
}

#[test]
fn test_no_valid_data_cache_key() {
    assert_eq!(
        no_valid_data_cache_key("7811"),
        "YahooProfileNoValidData:7811"
    );
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 visit");

    match visit("2330").await {
        Ok(e) => {
            dbg!(&e);
            tracing::debug!("{:#?}", e);
        }
        Err(why) => {
            tracing::debug!("Failed to visit because {:?}", why);
        }
    }

    tracing::debug!("結束 visit");
}
