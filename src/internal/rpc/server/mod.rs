use anyhow::Result;
use tonic::transport::{Identity, Server, ServerTlsConfig};

use crate::internal::{
    config::SETTINGS,
    logging,
    rpc::{control::control_server::ControlServer, server::control_service::ControlService},
};

pub mod control_service;

/// 啟動 GRPC Server
pub async fn start() -> Result<()> {
    if SETTINGS.system.grpc_use_port == 0 {
        return Ok(());
    }

    /*let addr = format!("0.0.0.0:{}", SETTINGS.system.grpc_use_port).parse()?;
    logging::info_file_async(format!("啟動 gRPC({:?}) 服務", addr));
    if !SETTINGS.system.ssl_cert_file.is_empty() && !SETTINGS.system.ssl_key_file.is_empty() {
        let cert = std::fs::read_to_string(&SETTINGS.system.ssl_cert_file)?;
        let key = std::fs::read_to_string(&SETTINGS.system.ssl_key_file)?;
        let identity = Identity::from_pem(cert, key);
        Server::builder()
            .tls_config(ServerTlsConfig::new().identity(identity))?
            .add_service(ControlServer::new(ControlService::default()))
            .serve(addr)
            .await?;
    } else {
        Server::builder()
            .add_service(ControlServer::new(ControlService::default()))
            .serve(addr)
            .await?;
    }*/

    let addr = format!("0.0.0.0:{}", SETTINGS.system.grpc_use_port).parse()?;

    /* let builder = Server::builder();
    let mut server =
        if !SETTINGS.system.ssl_cert_file.is_empty() && !SETTINGS.system.ssl_key_file.is_empty() {
            let cert_content = std::fs::read_to_string(&SETTINGS.system.ssl_cert_file)?;
            let key_content = std::fs::read_to_string(&SETTINGS.system.ssl_key_file)?;
            let identity = Identity::from_pem(cert_content, key_content);
            builder.tls_config(ServerTlsConfig::new().identity(identity))?
        } else {
            builder
        };

    server
        .add_service(ControlServer::new(ControlService::default()))
        .serve(addr)
        .await?;*/
    // 使用 tokio::spawn 啟動一個新的異步任務
    tokio::spawn(async move {
        let builder = Server::builder();
        let mut server = if !SETTINGS.system.ssl_cert_file.is_empty()
            && !SETTINGS.system.ssl_key_file.is_empty()
        {
            let cert_content = std::fs::read_to_string(&SETTINGS.system.ssl_cert_file)
                .expect("Failed to read ssl_cert_file");
            let key_content = std::fs::read_to_string(&SETTINGS.system.ssl_key_file)
                .expect("Failed to read ssl_key_file");
            let identity = Identity::from_pem(cert_content, key_content);
            builder
                .tls_config(ServerTlsConfig::new().identity(identity))
                .expect("Failed to set tls_config")
        } else {
            builder
        };

        server
            .add_service(ControlServer::new(ControlService::default()))
            .serve(addr)
            .await
            .expect("GRPC Server error");
    });

    logging::info_file_async(format!("啟動 gRPC({:?}) 服務", addr));

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::thread;

    use tokio::time::Duration;

    use super::*;

    #[tokio::test]
    async fn test_start() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 rpc::start()".to_string());

        tokio::spawn(start());
        thread::sleep(Duration::from_secs(10));
        /* match  start().await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to rpc::start() because: {:?}", why));
            }
        }*/
        logging::debug_file_async("結束 rpc::start()".to_string());
    }
}
