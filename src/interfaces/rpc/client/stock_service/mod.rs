//! Stock gRPC 客戶端服務實作。
//!
//! 提供向遠端 gRPC 服務（如 Go 服務）發送股票資訊的方法。

use anyhow::Result;
use tonic::{Request, Response};

use crate::interfaces::rpc::{
    client::{Grpc, get_client},
    stock::{StockInfoReply, StockInfoRequest},
};

impl Grpc {
    /// 將股票資訊更新通知發送至 Go 服務。
    ///
    /// 透過 gRPC `update_stock_info` 介面將最新的股票資訊同步至遠端服務。
    ///
    /// # Arguments
    ///
    /// * `request` - 包含股票詳細資訊的 `StockInfoRequest`。
    ///
    /// # Errors
    ///
    /// 如果 gRPC 調用失敗，則回傳錯誤。
    pub async fn update_stock_info(
        &self,
        request: StockInfoRequest,
    ) -> Result<Response<StockInfoReply>> {
        let mut client = self.stock.clone();
        Ok(client.update_stock_info(Request::new(request)).await?)
    }
}

/// 全域函數：將股票資訊推送至 Go 服務。
///
/// 此函數會自動取得或初始化全域 gRPC 客戶端，並發送請求。
///
/// # Arguments
///
/// * `request` - 包含股票詳細資訊的 `StockInfoRequest`。
pub async fn push_stock_info_to_go_service(
    request: StockInfoRequest,
) -> Result<Response<StockInfoReply>> {
    get_client().await?.update_stock_info(request).await
}

/// 把 gRPC 推送接上 `app::ports::StockInfoGateway` port 的 adapter。
///
/// 依 DDD 分層，app 層不應直接依賴 prost 產生的 `StockInfoRequest`（傳輸層
/// 細節）。app 只呼叫 `app::ports::push_stock_info`；由這個 adapter 負責
/// 「與傳輸無關的載體 → gRPC DTO」的轉換，並在 `main` 啟動時註冊。
pub struct GrpcStockInfoGateway;

#[async_trait::async_trait]
impl crate::app::ports::StockInfoGateway for GrpcStockInfoGateway {
    async fn push_stock_info(&self, info: crate::app::ports::StockInfoPush) -> Result<()> {
        // 在 adapter 內完成傳輸層 DTO 的組裝；prost 專屬欄位在這裡補預設值。
        let request = StockInfoRequest {
            stock_symbol: info.stock_symbol,
            name: info.name,
            stock_exchange_market_id: info.stock_exchange_market_id,
            stock_industry_id: info.stock_industry_id,
            net_asset_value_per_share: 0.0,
            suspend_listing: false,
        };
        push_stock_info_to_go_service(request).await?;
        Ok(())
    }
}

impl From<&crate::domain::registry::entity::Stock> for StockInfoRequest {
    fn from(stock: &crate::domain::registry::entity::Stock) -> Self {
        StockInfoRequest {
            stock_symbol: stock.symbol().0.clone(),
            name: stock.name().to_string(),
            stock_exchange_market_id: stock.market_id(),
            stock_industry_id: stock.industry_id(),
            net_asset_value_per_share:
                <rust_decimal::Decimal as rust_decimal::prelude::ToPrimitive>::to_f64(
                    &stock.net_asset_value_per_share(),
                )
                .unwrap_or(0.0),
            suspend_listing: stock.suspend_listing(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::infra::cache::SHARE;

    use super::*;

    /// 驗證股票資訊是否能成功推送至遠端 Go 服務。
    ///
    /// 此測試預設忽略，僅在需要手動驗證與遠端服務連線時使用。
    #[tokio::test]
    #[ignore]
    async fn test_push_stock_info_to_go_service() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 push_stock_info_to_go_service");
        let request = StockInfoRequest {
            stock_symbol: "7533967".to_string(),
            name: "tonic".to_string(),
            stock_exchange_market_id: 1,
            stock_industry_id: 2,
            net_asset_value_per_share: 1.235,
            suspend_listing: false,
        };

        match push_stock_info_to_go_service(request).await {
            Ok(response) => {
                tracing::debug!("response:{:#?}", response);
            }
            Err(why) => {
                tracing::debug!("Failed to push_stock_info_to_go_service because {:?}", why);
            }
        }
        tracing::debug!("結束 push_stock_info_to_go_service");
    }
}
