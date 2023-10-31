use anyhow::Result;
use tonic::{Request, Response, Status};

use crate::rpc::{
    basic::BaseResponse,
    control::{control_server::Control, ControlRequest, ControlResponse},
};

#[derive(Default)]
pub struct ControlService {}

#[tonic::async_trait]
impl Control for ControlService {
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

    #[tokio::test]
    async fn test_say_hello() {
        // Create the mock server
        let mock_service = ControlService::default();
        let mock_server = tonic::transport::Server::builder()
            .add_service(ControlServer::new(mock_service))
            .serve("127.0.0.1:50051".parse().unwrap());
        //.await .expect("Server failed");

        tokio::spawn(mock_server);

        // Wait a bit for server to be up. In real-world cases, you'd use a more robust mechanism.
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Use the service like you would against a real server
        let mut client = control::control_client::ControlClient::connect("http://127.0.0.1:50051")
            .await
            .expect("Failed to connect");

        let request = Request::new(ControlRequest {});

        let resp = client.control(request).await.expect("RPC Failed!");
        println!("message:{:?}", resp.into_inner().message)
        //assert_eq!(response.into_inner().message, "Hello Tonic!");
    }

    #[tokio::test]
    #[ignore]
    async fn test_control_request_to_server() {
        dotenv::dotenv().ok();
        let pem = std::fs::read_to_string(&SETTINGS.system.ssl_cert_file).unwrap();
        let ca = Certificate::from_pem(pem);

        let tls = ClientTlsConfig::new()
            .ca_certificate(ca)
            .domain_name("jiansoft.mooo.com");

        // Use the service like you would against a real server

        let channel = Channel::from_static("http://192.168.111.224:9001")
            .tls_config(tls)
            .unwrap()
            .connect()
            .await
            .expect("Failed to connect");

        let mut client = ControlClient::new(channel);

        /* let mut client = control::control_client::ControlClient::connect("http://127.0.0.1:9001")
        .await
        .expect("Failed to connect");*/

        let request = Request::new(ControlRequest {});

        let resp = client.control(request).await.expect("RPC Failed!");
        println!("message:{:?}", resp.into_inner().message)
        //assert_eq!(response.into_inner().message, "Hello Tonic!");
    }

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
