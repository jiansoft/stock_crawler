//! Control gRPC 服務實作。
//!
//! 提供系統控制與基本通訊測試功能，可用於健康檢查或確認連線狀態。

use anyhow::Result;
use tonic::{Request, Response, Status};

use crate::rpc::{
    basic::BaseResponse,
    control::{control_server::Control, ControlRequest, ControlResponse},
};

/// Control gRPC 服務。
///
/// 實作了 `Control` trait，處理來自客戶端的系統控制請求。
#[derive(Default)]
pub struct ControlService {}

#[tonic::async_trait]
impl Control for ControlService {
    /// 處理系統控制請求。
    ///
    /// 此方法目前主要用於測試連線，會記錄客戶端 IP 並回傳 200 OK。
    ///
    /// # Arguments
    ///
    /// * `req` - 包含 `ControlRequest` 的 gRPC 請求。
    ///
    /// # Returns
    ///
    /// 回傳 `ControlResponse`，其中包含基本的回應訊息與代碼。
    async fn control(
        &self,
        req: Request<ControlRequest>,
    ) -> Result<Response<ControlResponse>, Status> {
        if let Some(addr) = req.remote_addr() {
            println!("Client IP is: {}", addr);
        }
        println!("control receive request: {:?}", req);

        let response = ControlResponse {
            message: Some(BaseResponse {
                message: "Ok".to_string(),
                code: 200,
            }),
        };

        Ok(Response::new(response))
    }
}

#[cfg(test)]
mod tests {
    use tonic::transport::{Certificate, Channel, ClientTlsConfig};

    use crate::{
        config::SETTINGS,
        rpc::{
            control, control::control_client::ControlClient, control::control_server::ControlServer,
        },
    };

    use super::*;

    /// 驗證 Mock Server 控制介面。
    ///
    /// 在本地啟動一個臨時的 gRPC 伺服器並使用客戶端發送請求進行測試。
    #[tokio::test]
    async fn test_say_hello() {
        // Create the mock server
        let mock_service = ControlService::default();
        let mock_server = tonic::transport::Server::builder()
            .add_service(ControlServer::new(mock_service))
            .serve("127.0.0.1:50051".parse().unwrap());

        tokio::spawn(mock_server);

        // 等待伺服器啟動
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let mut client = control::control_client::ControlClient::connect("http://127.0.0.1:50051")
            .await
            .expect("Failed to connect");

        let request = Request::new(ControlRequest {});

        let resp = client.control(request).await.expect("RPC Failed!");
        println!("message:{:?}", resp.into_inner().message)
    }

    /// 驗證對外部 gRPC 伺服器發送 Control 請求。
    ///
    /// 此測試預設忽略，僅在需要手動驗證與特定伺服器連線時使用。
    #[tokio::test]
    #[ignore]
    async fn test_control_request_to_server() {
        dotenv::dotenv().ok();
        let pem = std::fs::read_to_string(&SETTINGS.system.ssl_cert_file).unwrap();
        let ca = Certificate::from_pem(pem);

        let tls = ClientTlsConfig::new()
            .ca_certificate(ca)
            .domain_name("jiansoft.mooo.com");

        let channel = Channel::from_static("http://192.168.111.224:9001")
            .tls_config(tls)
            .unwrap()
            .connect()
            .await
            .expect("Failed to connect");

        let mut client = ControlClient::new(channel);

        let request = Request::new(ControlRequest {});

        let resp = client.control(request).await.expect("RPC Failed!");
        println!("message:{:?}", resp.into_inner().message)
    }

    /// 驗證直接呼叫服務處理程序（不透過網路）。
    #[tokio::test]
    async fn test_control_request() {
        let c = ControlService::default();

        let request = Request::new(ControlRequest {});

        let response = c.control(request).await;

        match response {
            Ok(resp) => {
                println!("message:{:?}", resp.into_inner().message)
            }
            Err(e) => panic!("Test failed: {}", e),
        }
    }
}
