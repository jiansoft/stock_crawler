//! Axum web entry points.
//!
//! 這個模組負責啟動 HTTP 入口，並把各功能模組的 router 掛到同一個
//! Axum application 上。目前主要提供手動回補頁面與 API。
//!
//! ## 生命週期速覽
//!
//! `start` 只負責「準備並啟動」，真正的 accept loop 在背景 task。不同於 fire-and-forget，
//! 背景 task 的 [`WebServerHandle`] 會交回 `main` 保存。關機時 `main` 先透過 watch channel
//! 送出 `true`，Axum 停止收新 request 並排空既有 request，最後 `main` await handle。

use std::net::SocketAddr;

use anyhow::Result;
use tokio::{net::TcpListener, sync::watch, task::JoinHandle};

/// Backfill admin 的 Web UI 與 HTTP API。
pub mod backfill_admin;
/// 供內網服務讀取股票資料的版本化唯讀 API。
pub mod data_api;

/// 手動回補 Web 服務監聽位址的環境變數名稱。
const MANUAL_BACKFILL_WEB_ADDR: &str = "MANUAL_BACKFILL_WEB_ADDR";
/// 未設定環境變數時使用的本機監聽位址。
const DEFAULT_MANUAL_BACKFILL_WEB_ADDR: &str = "127.0.0.1:9002";

/// 可由主程式平順停止的 Web server 背景 task。
///
/// `JoinHandle` 的外層錯誤代表 task panic/被取消，內層 `Result` 則代表 Axum serve 錯誤；
/// 保留兩層可讓 `main` 在 log 中清楚區分「task 壞掉」與「server 回傳錯誤」。
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
    let app = backfill_admin::router().merge(data_api::router());
    // bind 必須在 spawn 前完成，讓 port 被占用等錯誤可以在啟動階段直接回報。
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("manual backfill web server listening on http://{}", addr);

    // 不在此處丟棄 JoinHandle。交回 main 後，程序才知道 server 何時真的停止，
    // 不會發生 main 已退出、runtime 把尚在處理 request 的 server 直接砍掉。
    Ok(tokio::spawn(run_server(listener, app, shutdown)))
}

/// 執行 Axum accept loop，並在收到關機訊號後排空既有 HTTP request。
///
/// `with_graceful_shutdown` 的語意不是立刻取消所有 handler；它會停止接受新連線，並等待
/// 已進入服務的 request 完成。Manual backfill handler 會另外把長工作交給 operation tracker，
/// 因此 handler 回應 job id 後，主程式仍知道後方真正的資料工作尚未結束。
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
