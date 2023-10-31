use anyhow::Result;
use tonic::{Request, Response};

use crate::rpc::{
    client::{get_client, Grpc},
    stock::{StockInfoReply, StockInfoRequest},
};

impl Grpc {
    /// 將 stock info 通知 go service
    pub async fn update_stock_info(
        &self,
        request: StockInfoRequest,
    ) -> Result<Response<StockInfoReply>> {
        let mut client = self.stock.clone();
        Ok(client.update_stock_info(Request::new(request)).await?)
    }
}

pub async fn push_stock_info_to_go_service(
    request: StockInfoRequest,
) -> Result<Response<StockInfoReply>> {
    get_client().await?.update_stock_info(request).await
}

#[cfg(test)]
mod tests {
    use crate::{internal::cache::SHARE, logging};

    use super::*;

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
