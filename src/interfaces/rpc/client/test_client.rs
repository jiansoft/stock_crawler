//! gRPC 測試客戶端。
//!
//! 提供一個簡單的測試工具，用於驗證本地或遠端 gRPC 伺服器的可用性與連線狀態。

use crate::core::config::SETTINGS;
use crate::interfaces::rpc::control::ControlRequest;
// 服務定義改名為 ControlService 後，tonic 產生的客戶端型別也更名
use crate::interfaces::rpc::control::control_service_client::ControlServiceClient;
use anyhow::Result;
use std::fs;
use tonic::transport::{Certificate, Channel, ClientTlsConfig};

/// 執行 gRPC 伺服器運行測試。
///
/// 此函數會嘗試連線至本地 gRPC 伺服器，發送一個 `ControlRequest` 請求，
/// 並驗證是否能收到正確的回應。此工具主要用於自動化測試或系統啟動後的自我檢查。
///
/// # 流程說明：
/// 1. 檢查設定檔中的 gRPC 埠號。
/// 2. 建立連線目標位址 (127.0.0.1)。
/// 3. 載入 SSL 憑證以進行 TLS 連線。
/// 4. 建立連線並設定 5 秒超時。
/// 5. 調用 `control` 方法。
/// 6. 記錄測試結果至日誌系統。
///
/// # Errors
///
/// 如果連線過程發生不可預期的錯誤（如憑證讀取失敗），則回傳錯誤。
pub async fn run_test() -> Result<()> {
    tracing::info!("開始 gRPC Server 運行測試...");

    let port = SETTINGS.system.grpc_use_port;
    if port == 0 {
        tracing::warn!("gRPC 埠號設定為 0，跳過測試");
        return Ok(());
    }

    // 建立連線目標 (改用 127.0.0.1 避免 localhost 解析延遲)
    let target = format!("https://127.0.0.1:{}", port);
    tracing::info!("正在連線至測試目標: {}", target);

    let cert_file = &SETTINGS.system.ssl_cert_file;
    let key_file = &SETTINGS.system.ssl_key_file;

    // 只有 cert 和 key 都設定時才使用 TLS，與伺服器的啟動條件保持一致
    let endpoint = if !cert_file.is_empty() && !key_file.is_empty() {
        let pem = fs::read_to_string(cert_file)?;
        let ca = Certificate::from_pem(pem);
        let domain = "jiansoft.ddns.net";
        let tls = ClientTlsConfig::new()
            .ca_certificate(ca)
            .domain_name(domain);
        tracing::info!("TLS 模式連線 [domain={}, cert={}]", domain, cert_file);
        Channel::from_shared(target.clone())?
            .tls_config(tls)?
            .connect_timeout(std::time::Duration::from_secs(5))
    } else {
        // 伺服器以 insecure 模式啟動，對應改用明文連線
        let plain_target = format!("http://127.0.0.1:{}", port);
        tracing::info!("Insecure 模式連線 [target={}]", plain_target);
        Channel::from_shared(plain_target)?.connect_timeout(std::time::Duration::from_secs(5))
    };

    match tokio::time::timeout(std::time::Duration::from_secs(6), endpoint.connect()).await {
        Ok(Ok(channel)) => {
            tracing::info!("gRPC 通道建立成功，準備發送 Request...");

            let mut client = ControlServiceClient::new(channel);
            let request = tonic::Request::new(ControlRequest {});

            match client.control(request).await {
                Ok(response) => {
                    tracing::info!("gRPC 測試成功！收到回應: {:?}", response.into_inner());
                }
                Err(e) => {
                    tracing::error!("gRPC 方法呼叫失敗: {:#}", e);
                }
            }
        }
        Ok(Err(e)) => {
            tracing::error!(
                "連線至 gRPC 伺服器失敗 [target={}]: {}",
                target,
                format_error_chain(&e)
            );
        }
        Err(_) => {
            tracing::error!("gRPC 連線測試超時 (超過 6 秒)");
        }
    }

    Ok(())
}

/// 將 `std::error::Error` 的完整 cause chain 串成一行，方便 log 閱讀。
///
/// 例如：`transport error: hyper error: connection reset by peer`
fn format_error_chain(e: &dyn std::error::Error) -> String {
    let mut msg = e.to_string();
    let mut source = e.source();
    while let Some(cause) = source {
        msg.push_str(": ");
        msg.push_str(&cause.to_string());
        source = cause.source();
    }
    msg
}
