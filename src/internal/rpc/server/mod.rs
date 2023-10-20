use anyhow::Result;
use std::net::SocketAddr;
use tonic::transport::{Identity, Server, ServerTlsConfig};

use crate::internal::{
    config::SETTINGS,
    logging,
    rpc::{
        control::control_server::ControlServer, server::control_service::ControlService,
        server::stock_service::StockService, stock::stock_server::StockServer,
    },
};

pub mod control_service;
pub mod stock_service;

/// 啟動 GRPC Server
pub async fn start() -> Result<()> {
    if SETTINGS.system.grpc_use_port == 0 {
        return Ok(());
    }

    let addr = format!("0.0.0.0:{}", SETTINGS.system.grpc_use_port).parse()?;

    // 使用 tokio::spawn 啟動一個新的異步任務
    tokio::spawn(async move {
        if let Err(why) = run_grpc_server(addr).await {
            logging::error_file_async(format!("gRPC伺服器錯誤: {}", why));
        }
    });

    logging::info_file_async(format!("啟動 gRPC({:?}) 服務", addr));

    Ok(())
}

async fn run_grpc_server(addr: SocketAddr) -> Result<()> {
    let builder = Server::builder();
    let mut server = match get_tls_config() {
        Some(config) => configure_tls(builder, config)?,
        None => builder,
    };

    Ok(server
        .add_service(ControlServer::new(ControlService::default()))
        .add_service(StockServer::new(StockService::default()))
        .serve(addr)
        .await?)
}

fn get_tls_config() -> Option<(String, String)> {
    if !SETTINGS.system.ssl_cert_file.is_empty() && !SETTINGS.system.ssl_key_file.is_empty() {
        Some((
            SETTINGS.system.ssl_cert_file.clone(),
            SETTINGS.system.ssl_key_file.clone(),
        ))
    } else {
        None
    }
}

fn configure_tls(builder: Server, (cert_file, key_file): (String, String)) -> Result<Server> {
    let cert_content = std::fs::read_to_string(cert_file)?;
    let key_content = std::fs::read_to_string(key_file)?;
    let identity = Identity::from_pem(cert_content, key_content);

    Ok(builder.tls_config(ServerTlsConfig::new().identity(identity))?)
}

#[cfg(test)]
mod tests {

    use tokio::time::Duration;

    use super::*;

    #[tokio::test]
    async fn test_start() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 rpc::start()".to_string());

        tokio::spawn(start());
        tokio::time::sleep(Duration::from_secs(10)).await;
        /* match  start().await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to rpc::start() because: {:?}", why));
            }
        }*/
        logging::debug_file_async("結束 rpc::start()".to_string());
    }
}
