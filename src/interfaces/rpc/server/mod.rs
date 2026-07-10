//! gRPC 伺服器模組。
//!
//! 負責 gRPC 伺服器的啟動、設定、TLS 憑證管理以及服務註冊。
//!
//! ## 生命週期速覽
//!
//! `start` 建立一個 Tonic 背景 task，並把 [`GrpcServerHandle`] 交給 `main`。收到 watch 關機
//! 訊號後，`serve_with_shutdown` 停止接受新 RPC，等待既有 RPC 返回，`main` 再 await handle。
//! `grpc_use_port == 0` 時回傳 `None`，代表設定上刻意停用，不是啟動失敗。

use std::{
    io::{BufReader, Cursor},
    net::SocketAddr,
};

use anyhow::Result;
use tokio::{sync::watch, task::JoinHandle};
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

/// 可由主程式平順停止的 gRPC server 背景 task。
///
/// 與 Web server 相同，外層 `JoinHandle` 表示 task 生命週期，內層 `Result` 表示 Tonic
/// server 本身的結果；兩層都必須由 `main` 檢查。
pub type GrpcServerHandle = JoinHandle<Result<()>>;

/// 啟動 gRPC 伺服器。
///
/// 根據設定檔中的埠號啟動伺服器。如果埠號為 0，則不啟動。
/// 伺服器會在背景任務中執行，並在收到 shutdown watch 訊號後停止接受新連線。
///
/// # Errors
///
/// 如果解析地址失敗或啟動過程中發生錯誤，將會回傳錯誤。
pub async fn start(shutdown: watch::Receiver<bool>) -> Result<Option<GrpcServerHandle>> {
    if SETTINGS.system.grpc_use_port == 0 {
        return Ok(None);
    }

    let addr = format!("0.0.0.0:{}", SETTINGS.system.grpc_use_port).parse()?;

    tracing::info!("啟動 gRPC({:?}) 服務", addr);

    // 不採 fire-and-forget：保存 JoinHandle 讓 main 在關機時等待 Tonic 完成既有 RPC。
    Ok(Some(tokio::spawn(async move {
        run_grpc_server(addr, shutdown).await
    })))
}

/// 運行 gRPC 伺服器實例。
///
/// 負責建立伺服器 Builder、套用 TLS 設定並註冊服務。
///
/// `shutdown` 是 watch receiver 的獨立 clone；它只讀取狀態，不會互相消耗訊號，
/// 因此 Web 與 gRPC 能同時看到同一次關機通知。
async fn run_grpc_server(addr: SocketAddr, shutdown: watch::Receiver<bool>) -> Result<()> {
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
        .serve_with_shutdown(addr, crate::core::shutdown::wait_for_shutdown(shutdown))
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
    let cert = match rustls_pki_types::CertificateDer::pem_reader_iter(&mut reader)
        .next()
        .transpose()
    {
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

        // app.json 設定了 TLS 憑證路徑，但 CI 等環境不會有實際的憑證檔
        // （fullchain.pem/privkey.pem 屬於機密，不進版控）。缺檔時 Tonic 會在
        // 啟動瞬間失敗，那是環境問題而非本測試要驗證的行為，因此先檢查並跳過。
        if let Some((cert_file, key_file)) = get_tls_config()
            && (!std::path::Path::new(&cert_file).exists()
                || !std::path::Path::new(&key_file).exists())
        {
            println!("跳過 test_start：TLS 憑證檔不存在（cert={cert_file}, key={key_file}）");
            return;
        }

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        if let Some(handle) = start(shutdown_rx).await.expect("gRPC server should start") {
            tokio::time::sleep(Duration::from_millis(100)).await;
            // send 失敗代表 receiver 已被 drop——也就是 server 沒等到關機訊號就
            // 提前結束（例如 port 被占用、bind 失敗）。此時改為 await task 取出
            // server 真正的錯誤原因回報，比只顯示 SendError 更容易定位問題。
            if shutdown_tx.send(true).is_err() {
                let result = handle.await.expect("gRPC server task should join");
                panic!("gRPC server 未收到關機訊號就提前結束：{result:?}");
            }
            handle
                .await
                .expect("gRPC server task should join")
                .expect("gRPC server should stop cleanly");
        }

        tracing::debug!("結束 rpc::server::test_start()");
    }
}
