//! gRPC 伺服器模組。
//!
//! 負責 gRPC 伺服器的啟動、設定、TLS 憑證管理以及服務註冊。

use std::{
    io::{BufReader, Cursor},
    net::SocketAddr,
};

use anyhow::Result;
use tonic::transport::{Identity, Server, ServerTlsConfig};

use crate::{
    core::config::SETTINGS,
    core::util,
    interfaces::rpc::{
        // 服務定義加上 Service 後綴後，tonic 產生的包裝型別也相應更名
        control::control_service_server::ControlServiceServer,
        manual_backfill::manual_backfill_service_server::ManualBackfillServiceServer,
        // 實作結構體已更名為 *ServiceImpl，避免與 tonic 產生 trait 同名
        server::control_service::ControlServiceImpl,
        server::manual_backfill_service::ManualBackfillServiceImpl,
        server::stock_service::StockServiceImpl,
        stock::stock_service_server::StockServiceServer,
    },
};

/// Control 服務實作模組。
pub mod control_service;
/// Manual backfill 服務實作模組。
pub mod manual_backfill_service;
/// Stock 服務實作模組。
pub mod stock_service;

/// 啟動 gRPC 伺服器。
///
/// 根據設定檔中的埠號啟動伺服器。如果埠號為 0，則不啟動。
/// 伺服器會在背景任務中執行，不會阻塞當前執行緒。
///
/// # Errors
///
/// 如果解析地址失敗或啟動過程中發生錯誤，將會回傳錯誤。
pub async fn start() -> Result<()> {
    if SETTINGS.system.grpc_use_port == 0 {
        return Ok(());
    }

    let addr = format!("0.0.0.0:{}", SETTINGS.system.grpc_use_port).parse()?;

    // 使用 tokio::spawn 啟動一個新的異步任務
    tokio::spawn(async move {
        if let Err(why) = run_grpc_server(addr).await {
            tracing::error!("gRPC伺服器錯誤: {}", why);
        }
    });

    tracing::info!("啟動 gRPC({:?}) 服務", addr);

    Ok(())
}

/// 運行 gRPC 伺服器實例。
///
/// 負責建立伺服器 Builder、套用 TLS 設定並註冊服務。
async fn run_grpc_server(addr: SocketAddr) -> Result<()> {
    tracing::info!("準備建立 gRPC 伺服器並監聽 {:?}", addr);
    let builder = Server::builder();
    let config = get_tls_config();

    if config.is_some() {
        tracing::info!("gRPC 伺服器將使用 TLS 設定啟動");
    } else {
        tracing::info!("gRPC 伺服器將使用非加密模式 (Insecure) 啟動");
    }

    let mut server = match config {
        Some(config) => configure_tls(builder, config)?,
        None => builder,
    };

    tracing::info!("gRPC 伺服器正在 {:?} 開始服務...", addr);
    let result = server
        // 使用更新後的 Server 包裝型別（名稱含 Service 後綴）
        .add_service(ControlServiceServer::new(ControlServiceImpl::default()))
        .add_service(ManualBackfillServiceServer::new(
            ManualBackfillServiceImpl::default(),
        ))
        .add_service(StockServiceServer::new(StockServiceImpl::default()))
        .serve(addr)
        .await;

    match &result {
        Ok(_) => tracing::info!("gRPC 伺服器在 {:?} 正常停止", addr),
        Err(why) => tracing::error!("gRPC 伺服器運行中斷 ({:?}): {}", addr, why),
    }

    Ok(result?)
}

/// 從設定檔中取得 TLS 憑證與金鑰路徑。
///
/// 如果設定檔中未設定憑證或金鑰路徑，則回傳 `None`。
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

/// 將 TLS 設定套用到 gRPC 伺服器 Builder。
///
/// # Arguments
///
/// * `builder` - tonic 伺服器 Builder。
/// * `cert_file` - 憑證檔案路徑。
/// * `key_file` - 金鑰檔案路徑。
///
/// # Errors
///
/// 如果讀取檔案失敗或 TLS 設定無效，則回傳錯誤。
fn configure_tls(builder: Server, (cert_file, key_file): (String, String)) -> Result<Server> {
    util::ensure_rustls_crypto_provider();

    tracing::info!("正在載入 SSL 憑證檔案: {}", cert_file);
    tracing::info!("正在載入 SSL 金鑰檔案: {}", key_file);

    let cert_content = std::fs::read_to_string(&cert_file).map_err(|why| {
        tracing::error!("讀取憑證檔案失敗 ({}): {}", cert_file, why);
        why
    })?;
    let key_content = std::fs::read_to_string(&key_file).map_err(|why| {
        tracing::error!("讀取金鑰檔案失敗 ({}): {}", key_file, why);
        why
    })?;
    let cert_info = describe_certificate(&cert_content);

    tracing::info!(
        "SSL 載入成功 - 憑證: {} bytes, 資訊: [{}], 金鑰: {} bytes",
        cert_content.len(),
        cert_info,
        key_content.len()
    );

    let identity = Identity::from_pem(cert_content, key_content);

    Ok(builder.tls_config(ServerTlsConfig::new().identity(identity))?)
}

/// 解析 PEM/X.509 憑證資訊。
///
/// 用於日誌記錄，提供憑證的 Subject 與過期時間等資訊。
/// 使用純 Rust 實作，不依賴外部 openssl 指令。
fn describe_certificate(cert_pem: &str) -> String {
    use rustls_pki_types::pem::PemObject;

    // 建立一個讀取器，將 PEM 格式的憑證資料包裝成 BufReader
    let mut reader = BufReader::new(Cursor::new(cert_pem.as_bytes()));
    // 使用 rustls-pki-types 的 pem_reader_iter 迭代解析 PEM 資料，並取出第一個憑證區塊
    let cert = match rustls_pki_types::CertificateDer::pem_reader_iter(&mut reader).next().transpose() {
        Ok(Some(cert)) => cert, // 成功解析出憑證
        Ok(None) => return "PEM 中找不到 CERTIFICATE 區塊".to_string(), // 找不到憑證區塊
        Err(why) => return format!("憑證 PEM 解析失敗: {}", why), // 解析發生錯誤
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

    /// 測試 gRPC 伺服器啟動流程。
    #[tokio::test]
    async fn test_start() {
        dotenvy::dotenv().ok();
        tracing::debug!("開始 rpc::server::test_start()");

        tokio::spawn(start());
        tokio::time::sleep(Duration::from_secs(10)).await;

        tracing::debug!("結束 rpc::server::test_start()");
    }
}
