use std::{
    io::{BufReader, Cursor},
    net::SocketAddr,
};

use anyhow::Result;
use tonic::transport::{Identity, Server, ServerTlsConfig};

use crate::{
    config::SETTINGS,
    logging,
    rpc::{
        control::control_server::ControlServer, server::control_service::ControlService,
        server::stock_service::StockService, stock::stock_server::StockServer,
    },
    util,
};

/// Control 服務實作。
pub mod control_service;
/// Stock 服務實作。
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
    logging::info_file_async(format!("準備建立 gRPC 伺服器並監聽 {:?}", addr));
    let builder = Server::builder();
    let config = get_tls_config();

    if config.is_some() {
        logging::info_file_async("gRPC 伺服器將使用 TLS 設定啟動");
    } else {
        logging::info_file_async("gRPC 伺服器將使用非加密模式 (Insecure) 啟動");
    }

    let mut server = match config {
        Some(config) => configure_tls(builder, config)?,
        None => builder,
    };

    logging::info_file_async(format!("gRPC 伺服器正在 {:?} 開始服務...", addr));
    let result = server
        .add_service(ControlServer::new(ControlService::default()))
        .add_service(StockServer::new(StockService::default()))
        .serve(addr)
        .await;

    match &result {
        Ok(_) => logging::info_file_async(format!("gRPC 伺服器在 {:?} 正常停止", addr)),
        Err(why) => logging::error_file_async(format!("gRPC 伺服器運行中斷 ({:?}): {}", addr, why)),
    }

    Ok(result?)
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

/// 將 TLS 設定套用到 gRPC server builder。
fn configure_tls(builder: Server, (cert_file, key_file): (String, String)) -> Result<Server> {
    util::ensure_rustls_crypto_provider();

    logging::info_file_async(format!("正在載入 SSL 憑證檔案: {}", cert_file));
    logging::info_file_async(format!("正在載入 SSL 金鑰檔案: {}", key_file));

    let cert_content = std::fs::read_to_string(&cert_file).map_err(|why| {
        logging::error_file_async(format!("讀取憑證檔案失敗 ({}): {}", cert_file, why));
        why
    })?;
    let key_content = std::fs::read_to_string(&key_file).map_err(|why| {
        logging::error_file_async(format!("讀取金鑰檔案失敗 ({}): {}", key_file, why));
        why
    })?;
    let cert_info = describe_certificate(&cert_content);

    logging::info_file_async(format!(
        "SSL 載入成功 - 憑證: {} bytes, 資訊: [{}], 金鑰: {} bytes",
        cert_content.len(),
        cert_info,
        key_content.len()
    ));

    let identity = Identity::from_pem(cert_content, key_content);

    Ok(builder.tls_config(ServerTlsConfig::new().identity(identity))?)
}

/// 使用純 Rust 解析 PEM/X.509 憑證資訊，避免依賴外部 openssl 指令。
fn describe_certificate(cert_pem: &str) -> String {
    let mut reader = BufReader::new(Cursor::new(cert_pem.as_bytes()));
    let cert = match rustls_pemfile::certs(&mut reader).next().transpose() {
        Ok(Some(cert)) => cert,
        Ok(None) => return "PEM 中找不到 CERTIFICATE 區塊".to_string(),
        Err(why) => return format!("憑證 PEM 解析失敗: {}", why),
    };

    let parsed = match x509_parser::parse_x509_certificate(cert.as_ref()) {
        Ok((_, parsed)) => parsed,
        Err(why) => return format!("X.509 憑證解析失敗: {}", why),
    };

    format!(
        "subject={}, not_after={}",
        parsed.subject(),
        parsed.validity().not_after
    )
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
