//! Axum web 入口的測試。

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
