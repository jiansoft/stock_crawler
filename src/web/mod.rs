//! Axum web entry points.
//!
//! 這個模組負責啟動 HTTP 入口，並把各功能模組的 router 掛到同一個
//! Axum application 上。目前主要提供手動回補頁面與 API。

use std::net::SocketAddr;

use anyhow::Result;
use tokio::net::TcpListener;

use crate::logging;

/// Backfill admin 的 Web UI 與 HTTP API。
pub mod backfill_admin;

/// 手動回補 Web 服務監聽位址的環境變數名稱。
const MANUAL_BACKFILL_WEB_ADDR: &str = "MANUAL_BACKFILL_WEB_ADDR";
/// 未設定環境變數時使用的本機監聽位址。
const DEFAULT_MANUAL_BACKFILL_WEB_ADDR: &str = "127.0.0.1:9002";

/// 在背景 task 啟動手動回補 Web server。
///
/// 啟動流程：
/// 1. 讀取 `MANUAL_BACKFILL_WEB_ADDR`，未設定時使用 `127.0.0.1:9002`。
/// 2. 建立 manual backfill router。
/// 3. 用 `tokio::spawn` 在背景綁定位址並啟動 Axum server。
/// 4. bind 或 serve 失敗時寫入 log，避免主流程被背景 HTTP 服務中斷。
pub async fn start() -> Result<()> {
    // 解析監聽位址；格式錯誤時讓呼叫端在啟動期直接得到錯誤。
    let addr = std::env::var(MANUAL_BACKFILL_WEB_ADDR)
        .unwrap_or_else(|_| DEFAULT_MANUAL_BACKFILL_WEB_ADDR.to_string())
        .parse::<SocketAddr>()?;
    // 建立目前 Web 服務需要的所有路由。
    let app = backfill_admin::router();

    // Web server 是輔助入口，因此放到背景 task，不阻塞主程式後續排程。
    tokio::spawn(async move {
        match TcpListener::bind(addr).await {
            Ok(listener) => {
                logging::info_file_async(format!(
                    "manual backfill web server listening on http://{}",
                    addr
                ));

                // Axum serve 正常情況會持續執行；若返回錯誤，記錄原因供維運追查。
                if let Err(why) = axum::serve(listener, app).await {
                    logging::error_file_async(format!(
                        "manual backfill web server stopped with error: {}",
                        why
                    ));
                }
            }
            Err(why) => {
                // 背景服務 bind 失敗時只寫 log，避免影響既有 gRPC/排程流程。
                logging::error_file_async(format!(
                    "manual backfill web server bind failed on {}: {}",
                    addr, why
                ));
            }
        }
    });

    Ok(())
}
