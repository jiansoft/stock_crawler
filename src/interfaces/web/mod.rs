//! Axum web entry points.
//!
//! 這個模組負責啟動 HTTP 入口，並把各功能模組的 router 掛到同一個
//! Axum application 上。目前主要提供手動回補頁面與 API。

use std::net::SocketAddr;

use anyhow::Result;
use tokio::{net::TcpListener, sync::watch, task::JoinHandle};

/// Backfill admin 的 Web UI 與 HTTP API。
pub mod backfill_admin;

/// 手動回補 Web 服務監聽位址的環境變數名稱。
const MANUAL_BACKFILL_WEB_ADDR: &str = "MANUAL_BACKFILL_WEB_ADDR";
/// 未設定環境變數時使用的本機監聽位址。
const DEFAULT_MANUAL_BACKFILL_WEB_ADDR: &str = "127.0.0.1:9002";

/// 可由主程式平順停止的 Web server 背景 task。
pub type WebServerHandle = JoinHandle<Result<()>>;

/// 在背景 task 啟動手動回補 Web server。
///
/// 啟動流程：
/// 1. 讀取 `MANUAL_BACKFILL_WEB_ADDR`，未設定時使用 `127.0.0.1:9002`。
/// 2. 建立 manual backfill router。
/// 3. 在回傳前完成 bind，確保啟動錯誤能傳回主程式。
/// 4. 背景 server 收到 shutdown watch 訊號後停止接受新連線，並等待既有 request 完成。
pub async fn start(shutdown: watch::Receiver<bool>) -> Result<WebServerHandle> {
    // 解析監聽位址；格式錯誤時讓呼叫端在啟動期直接得到錯誤。
    let addr = std::env::var(MANUAL_BACKFILL_WEB_ADDR)
        .unwrap_or_else(|_| DEFAULT_MANUAL_BACKFILL_WEB_ADDR.to_string())
        .parse::<SocketAddr>()?;
    // 建立目前 Web 服務需要的所有路由。
    let app = backfill_admin::router();
    // bind 必須在 spawn 前完成，讓 port 被占用等錯誤可以在啟動階段直接回報。
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("manual backfill web server listening on http://{}", addr);

    // 將 JoinHandle 交回 main，關機時才能確認 server 已完成連線排空。
    Ok(tokio::spawn(run_server(listener, app, shutdown)))
}

/// 執行 Axum accept loop，並在收到關機訊號後排空既有 HTTP request。
async fn run_server(
    listener: TcpListener,
    app: axum::Router,
    shutdown: watch::Receiver<bool>,
) -> Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(crate::core::shutdown::wait_for_shutdown(shutdown))
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 驗證 Axum server 收到 watch 關機訊號後會正常結束 accept loop。
    #[tokio::test]
    async fn web_server_stops_after_shutdown_signal() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = tokio::spawn(run_server(listener, backfill_admin::router(), shutdown_rx));

        shutdown_tx
            .send(true)
            .expect("web shutdown receiver should exist");
        handle
            .await
            .expect("web server task should join")
            .expect("web server should stop cleanly");
    }
}
