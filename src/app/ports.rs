//! Application 層對外部服務的抽象介面（ports）。
//!
//! ## 為什麼需要這個模組
//!
//! 依 DDD 分層，`app`（use case）不應該直接依賴 `interfaces` 層的
//! gRPC 產生碼（prost DTO）——那是傳輸層細節，一旦 proto 變更或想改用
//! HTTP/訊息佇列推送，所有 use case 都得跟著改，也無法在單元測試中替換。
//!
//! ## 解法：port / adapter
//!
//! - 這裡定義與傳輸層無關的資料載體與 gateway trait（port）。
//! - 具體實作（gRPC client）由 `interfaces::rpc::client` 提供（adapter），
//!   並在 `main` 啟動時透過 [`register_stock_info_gateway`] 注入。
//! - 未註冊時（例如單元測試）推送會被略過並記 warning，不影響主流程。

use std::sync::{Arc, OnceLock};

use anyhow::Result;
use async_trait::async_trait;

/// 與傳輸層無關的股票基本資料載體。
///
/// 欄位刻意只用基本型別，不引用任何 prost 產生的 DTO。
#[derive(Debug, Clone)]
pub struct StockInfoPush {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 交易市場別 ID。
    pub stock_exchange_market_id: i32,
    /// 產業分類 ID。
    pub stock_industry_id: i32,
}

/// 將股票基本資料推送到外部服務的 gateway（port）。
#[async_trait]
pub trait StockInfoGateway: Send + Sync {
    /// 推送單筆股票基本資料。
    async fn push_stock_info(&self, info: StockInfoPush) -> Result<()>;
}

/// 全域 gateway 實例。`OnceLock` 保證只會被成功註冊一次。
static STOCK_INFO_GATEWAY: OnceLock<Arc<dyn StockInfoGateway>> = OnceLock::new();

/// 註冊股票資料推送 gateway，應於 `main` 啟動流程呼叫一次。
pub fn register_stock_info_gateway(gateway: Arc<dyn StockInfoGateway>) {
    if STOCK_INFO_GATEWAY.set(gateway).is_err() {
        tracing::warn!("stock info gateway already registered; duplicate registration ignored");
    }
}

/// 透過已註冊的 gateway 推送股票基本資料。
///
/// 尚未註冊時（例如單元測試）記 warning 後直接返回——
/// 推送屬「盡力同步」性質，不應讓主要事件流程失敗。
pub async fn push_stock_info(info: StockInfoPush) {
    match STOCK_INFO_GATEWAY.get() {
        Some(gateway) => {
            let symbol = info.stock_symbol.clone();
            if let Err(why) = gateway.push_stock_info(info).await {
                tracing::error!("Failed to push stock info for {symbol} because {why:?}");
            }
        }
        None => {
            tracing::warn!(
                "stock info gateway not registered; push skipped for {}",
                info.stock_symbol
            );
        }
    }
}
