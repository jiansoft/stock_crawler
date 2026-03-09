use std::sync::Arc;

use anyhow::Result;
use once_cell::sync::Lazy;
use tokio::{fs, sync::OnceCell as TokioOnceCell};
use tonic::transport::{Certificate, Channel, ClientTlsConfig};

use crate::{config::SETTINGS, rpc::stock::stock_client::StockClient};

/// Stock 服務 client。
pub mod stock_service;
/// gRPC 測試 client。
pub mod test_client;

/// 全域 gRPC client lazy 容器。
static GRPC: Lazy<Arc<TokioOnceCell<Grpc>>> = Lazy::new(|| Arc::new(TokioOnceCell::new()));

/// 已初始化的 gRPC client 集合。
struct Grpc {
    stock: StockClient<Channel>,
}

impl Grpc {
    /// 依設定檔建立帶 TLS 的 gRPC client。
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
}

/// 取得全域共用的 gRPC client。
async fn get_client() -> Result<&'static Grpc> {
    GRPC.get_or_try_init(|| async { Grpc::new().await }).await
}
