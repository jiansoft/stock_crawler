pub mod pb {
    include!("stock.rs");
}

use crate::internal::{
    config::SETTINGS,
    rpc::pb::{stock_client::StockClient, StockInfoReply, StockInfoRequest},
};
use anyhow::*;
use once_cell::sync::Lazy;
use std::{result::Result::Ok, sync::Arc};
use tokio::{fs, sync::OnceCell as TokioOnceCell};
use tonic::{
    transport::{Certificate, Channel, ClientTlsConfig},
    Request, Response,
};

static GRPC: Lazy<Arc<TokioOnceCell<Grpc>>> = Lazy::new(|| Arc::new(TokioOnceCell::new()));

struct Grpc {
    stock: StockClient<Channel>,
}

impl Grpc {
    pub async fn new() -> Result<Self> {
        let pem = fs::read_to_string(&SETTINGS.rpc.go_service.tls_cert_file).await?;
        let ca = Certificate::from_pem(pem);
        let tls = ClientTlsConfig::new()
            .ca_certificate(ca)
            .domain_name(&SETTINGS.rpc.go_service.domain_name);
        let channel = Channel::from_static(&SETTINGS.rpc.go_service.target)
            .tls_config(tls)?
            .connect()
            .await?;
        let client = StockClient::new(channel);
        Ok(Grpc { stock: client })
    }

    /// 將 stock info 通知 go service
    pub async fn update_stock_info(
        &self,
        request: StockInfoRequest,
    ) -> Result<Response<StockInfoReply>> {
        let mut client = self.stock.clone();
        Ok(client.update_stock_info(Request::new(request)).await?)
    }
}

async fn get_client() -> Result<&'static Grpc> {
    GRPC.get_or_try_init(|| async { Grpc::new().await }).await
}

pub async fn push_stock_info_to_go_service(
    request: StockInfoRequest,
) -> Result<Response<StockInfoReply>> {
    get_client().await?.update_stock_info(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::cache::SHARE;
    use crate::internal::logging;

    #[tokio::test]
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
