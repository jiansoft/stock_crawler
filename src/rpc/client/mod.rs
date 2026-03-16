//! gRPC 客戶端模組。
//!
//! 負責管理與外部 gRPC 服務（如 Go 服務）的連線與通訊。
//! 提供全域共用的客戶端實例。

use std::sync::Arc;

use anyhow::Result;
use once_cell::sync::Lazy;
use tokio::{fs, sync::OnceCell as TokioOnceCell};
use tonic::transport::{Certificate, Channel, ClientTlsConfig};

use crate::{config::SETTINGS, rpc::stock::stock_client::StockClient};

/// Stock 服務客戶端封裝。
pub mod stock_service;
/// gRPC 測試客戶端。
pub mod test_client;

/// 全域 gRPC 客戶端延遲初始化容器。
///
/// 使用 `Lazy` 與 `TokioOnceCell` 確保全域只有一個已初始化的 `Grpc` 實例。
static GRPC: Lazy<Arc<TokioOnceCell<Grpc>>> = Lazy::new(|| Arc::new(TokioOnceCell::new()));

/// 已初始化的 gRPC 客戶端集合。
///
/// 包含所有對外連線的 gRPC 客戶端實例。
pub struct Grpc {
    /// Stock 服務的 gRPC 客戶端。
    pub stock: StockClient<Channel>,
}

impl Grpc {
    /// 依據設定檔建立帶 TLS 的 gRPC 客戶端。
    ///
    /// 讀取 TLS 憑證並建立與遠端伺服器的安全連線。
    ///
    /// # Errors
    ///
    /// 如果讀取憑證檔案失敗或連線失敗，則回傳錯誤。
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

/// 取得全域共用的 gRPC 客戶端實例。
///
/// 第一次調用時會進行初始化，之後調用則直接回傳快取的實例。
async fn get_client() -> Result<&'static Grpc> {
    GRPC.get_or_try_init(|| async { Grpc::new().await }).await
}
