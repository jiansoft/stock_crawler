use tonic::transport::{Certificate, Channel, ClientTlsConfig};
use crate::config::SETTINGS;
use crate::logging;
use crate::rpc::control::control_client::ControlClient;
use crate::rpc::control::ControlRequest;
use anyhow::Result;
use std::fs;

/// 測試 gRPC 伺服器是否正常運行的客戶端工具
pub async fn run_test() -> Result<()> {
    logging::info_file_async("開始 gRPC Server 運行測試...");

    let port = SETTINGS.system.grpc_use_port;
    if port == 0 {
        logging::warn_file_async("gRPC 埠號設定為 0，跳過測試");
        return Ok(());
    }

    // 建立連線目標 (改用 127.0.0.1 避免 localhost 解析延遲)
    let target = format!("https://127.0.0.1:{}", port);
    logging::info_file_async(format!("正在連線至測試目標: {}", target));

    // 設定 TLS (使用與伺服器相同的憑證進行驗證)
    let cert_file = &SETTINGS.system.ssl_cert_file;
    if cert_file.is_empty() {
        logging::warn_file_async("未設定 SSL 憑證，無法進行 TLS 測試");
        return Ok(());
    }

    let pem = fs::read_to_string(cert_file)?;
    let ca = Certificate::from_pem(pem);
    
    let domain = "jiansoft.ddns.net";

    let tls = ClientTlsConfig::new()
        .ca_certificate(ca)
        .domain_name(domain);

    // 建立 Endpoint 對象
    let endpoint = Channel::from_shared(target)?
        .tls_config(tls)?
        .connect_timeout(std::time::Duration::from_secs(5)); // 設定 5 秒連線超時

    // 直接在 timeout 中呼叫 connect()
    match tokio::time::timeout(std::time::Duration::from_secs(6), endpoint.connect()).await {
        Ok(Ok(channel)) => {
            logging::info_file_async("gRPC 通道建立成功，準備發送 Request...");
            
            let mut client = ControlClient::new(channel);
            let request = tonic::Request::new(ControlRequest {});

            match client.control(request).await {
                Ok(response) => {
                    logging::info_file_async(format!(
                        "gRPC 測試成功！收到回應: {:?}",
                        response.into_inner()
                    ));
                }
                Err(e) => {
                    logging::error_file_async(format!("gRPC 方法呼叫失敗: {}", e));
                }
            }
        }
        Ok(Err(e)) => {
            logging::error_file_async(format!("連線至 gRPC 伺服器失敗 (可能是 TLS 握手錯誤或過期): {}", e));
        }
        Err(_) => {
            logging::error_file_async("gRPC 連線測試超時 (超過 6 秒)");
        }
    }

    Ok(())
}
