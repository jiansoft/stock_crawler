//! 各爬蟲連外 #[ignore] 測試共用的輔助函式：呼叫真實來源並把結果印出來，供手動檢查。

use anyhow::Result;

use super::StockInfo;

/// 呼叫 `S::get_stock_price` 並印出結果（失敗只記錄）。
pub(crate) async fn log_stock_price_test<S>(stock_symbol: &str)
where
    S: StockInfo,
{
    tracing::debug!("開始 get_stock_price");

    match S::get_stock_price(stock_symbol).await {
        Ok(price) => {
            dbg!(&price);
            tracing::debug!("price : {:#?}", price);
        }
        Err(why) => {
            tracing::debug!("Failed to get_stock_price because {:?}", why);
        }
    }

    tracing::debug!("結束 get_stock_price");
}

/// 呼叫查詢對外 IP 的 `visit` 並印出結果（失敗只記錄）。
pub(crate) async fn log_public_ip_visit_test<F, Fut>(visit: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    match visit().await {
        Ok(ip) => {
            dbg!(ip);
        }
        Err(why) => {
            tracing::error!("Failed to get because {:?}", why);
        }
    }
}
