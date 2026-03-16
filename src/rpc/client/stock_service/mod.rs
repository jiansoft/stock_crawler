//! Stock gRPC 客戶端服務實作。
//!
//! 提供向遠端 gRPC 服務（如 Go 服務）發送股票資訊的方法。

use anyhow::Result;
use tonic::{Request, Response};

use crate::rpc::{
    client::{get_client, Grpc},
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

#[cfg(test)]
mod tests {
    use crate::{cache::SHARE, logging};

    use super::*;

    /// 驗證股票資訊是否能成功推送至遠端 Go 服務。
    ///
    /// 此測試預設忽略，僅在需要手動驗證與遠端服務連線時使用。
    #[tokio::test]
    #[ignore]
    async fn test_push_stock_info_to_go_service() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 push_stock_info_to_go_service".to_string());
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
                logging::debug_file_async(format!("response:{:#?}", response));
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to push_stock_info_to_go_service because {:?}",
                    why
                ));
            }
        }
        logging::debug_file_async("結束 push_stock_info_to_go_service".to_string());
    }
}
