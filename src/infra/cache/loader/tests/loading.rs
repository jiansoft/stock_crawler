//! 從資料庫載入快取的整合測試。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_get_industry_name() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    assert_eq!(SHARE.get_industry_name(1), Some("水泥工業".to_string()));
    assert_eq!(SHARE.get_industry_name(2), Some("食品工業".to_string()));
    assert_eq!(SHARE.get_industry_name(99), Some("未分類".to_string()));
    assert_eq!(SHARE.get_industry_name(100), None);
}

/// 驗證 `load_required` 的必要/非必要快取合約。
///
/// 不同環境的預期：
/// - 資料庫不可用：應回傳 `Err`（必要快取查詢失敗），而不是默默成功。
/// - 資料庫可用且 stocks 有資料（本機）：`Ok` 且 `stock_count > 0`。
/// - 資料庫可用但 stocks 為空（CI 只建 schema 未 seed）：應回傳 `Err`
///   且錯誤訊息點名股票主檔——這正是「空的核心快取要 fail fast」的行為。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_load_required_contract() {
    dotenvy::dotenv().ok();

    match SHARE.load_required().await {
        Ok(report) => {
            // 成功即代表必要快取保證成立：股票主檔非空。
            assert!(report.stock_count > 0);
            println!(
                "load_required ok: stocks={}, quotes={}, degraded={:?}",
                report.stock_count, report.last_trading_day_quote_count, report.degraded
            );
        }
        Err(why) => {
            // 失敗必須可歸因於必要快取（連線失敗或股票主檔為空），
            // 錯誤訊息應點名失敗環節，方便啟動時定位。
            let msg = format!("{why:#}");
            assert!(
                msg.contains("股票主檔") || msg.contains("最後交易日報價"),
                "unexpected error: {msg}"
            );
            println!("load_required err (acceptable in this environment): {msg}");
        }
    }
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_load() {
    dotenvy::dotenv().ok();

    SHARE.load().await;

    let mut loop_count = 10;
    for e in SHARE.indices.read().unwrap().iter() {
        if loop_count < 0 {
            break;
        }
        tracing::info!("indices e.date {:?} e.index {:?}", e.1.date, e.1.index);
        loop_count -= 1;
    }

    loop_count = 10;
    for (k, v) in SHARE.stocks.read().unwrap().iter() {
        if loop_count < 0 {
            break;
        }
        tracing::info!("stock {} name {}", k, v.name());
        loop_count -= 1;
    }

    loop_count = 10;
    for (k, v) in SHARE.last_trading_day_quotes.read().unwrap().iter() {
        if loop_count < 0 {
            break;
        }
        tracing::info!("security_code {} closing_price {}", k, v.closing_price);
        loop_count -= 1;
    }

    for (k, v) in SHARE.industries.iter() {
        tracing::info!("name {}  category {}", k, v);
    }

    // 由於 guard 的 mutable 借用生命週期限制，此處無法直接合併 nested if let
    #[allow(clippy::collapsible_if)]
    if let Ok(mut guard) = SHARE.quote_history_records.write() {
        if let Some(qhr) = guard.get_mut("2330") {
            qhr.minimum_price = Decimal::from(1);
            qhr.maximum_price = Decimal::from(2);
        }
    }

    for (k, v) in SHARE.quote_history_records.read().unwrap().iter() {
        if k == "2330" {
            dbg!(v);
        }
    }
}
