//! 啟動第 15 步：等待 OS 關機訊號，依五道閘門平順停止。

use std::error::Error;

// Unix 平台上的細粒度訊號（SIGINT = Ctrl+C、SIGTERM = 系統/容器關閉）。
// Windows 上沒有 POSIX 訊號機制，因此用 cfg(unix) 限制，避免編譯錯誤。
// `unix_signal` 重命名以避免與模組名稱 `signal` 衝突。
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal as unix_signal};

use stock_crawler::{app, core};

use super::startup::Running;

/// 在 Unix 平台等待 `SIGINT`（Ctrl+C）或 `SIGTERM`（Docker/systemd 關閉訊號）。
///
/// `tokio::select!` 同時 await 兩個訊號流，哪個先到就哪個分支觸發退出；
/// 另一個分支對應的 future 會被自動取消（drop），不造成資源洩漏。
///
/// 若 Unix 訊號建立失敗（極罕見，通常是 fd 耗盡），向上回傳錯誤並中止服務，
/// 避免程序在無法接收關機訊號的狀態下繼續運作。
#[cfg(unix)]
async fn shutdown_signal() -> Result<(), Box<dyn Error>> {
    let mut sigint = unix_signal(SignalKind::interrupt())?;
    let mut sigterm = unix_signal(SignalKind::terminate())?;

    tokio::select! {
        _ = sigint.recv() => {}
        _ = sigterm.recv() => {}
    }

    Ok(())
}

/// 在非 Unix 平台等待 `Ctrl+C` 關機訊號。
///
/// Windows 沒有 POSIX SIGTERM，因此使用 Tokio 提供的跨平台 Ctrl+C future。
#[cfg(not(unix))]
async fn shutdown_signal() -> Result<(), Box<dyn Error>> {
    tokio::signal::ctrl_c().await?;
    Ok(())
}

/// 等待單一 server task 完成平順關機，逾時時才強制取消該 accept loop。
///
/// Server 內部已先收到 shutdown 訊號並停止接受新連線；此 timeout 只用來避免
/// 異常或永不結束的連線讓整個程序永久卡在關機階段。
async fn wait_for_server_shutdown(
    server_name: &str,
    mut handle: tokio::task::JoinHandle<anyhow::Result<()>>,
    timeout: tokio::time::Duration,
) {
    match tokio::time::timeout(timeout, &mut handle).await {
        Ok(Ok(Ok(()))) => tracing::info!(server = server_name, "server graceful shutdown done"),
        Ok(Ok(Err(why))) => {
            tracing::error!(server = server_name, error = %why, "server stopped with error")
        }
        Ok(Err(why)) => {
            tracing::error!(server = server_name, error = %why, "server task join failed")
        }
        Err(_) => {
            tracing::warn!(
                server = server_name,
                "server graceful shutdown timed out; aborting task"
            );
            handle.abort();
            let _ = handle.await;
        }
    }
}

/// 等待 OS 關機訊號後依序停止伺服器、排程、背景工作與價格 task。
///
/// # Errors
///
/// 無法建立 Unix 訊號監聽時回傳錯誤。
pub(crate) async fn wait_and_stop(running: Running) -> Result<(), Box<dyn Error>> {
    // ── 15. 等待 OS 關機訊號 ─────────────────────────────────────────────────
    //
    // 新進開發者可把下方關機程式理解成五道閘門：
    //
    //   1. Server 閘門：先拒絕新 request，避免關機期間工作持續增加。
    //   2. Scheduler 閘門：停止 cron ticker，避免又啟動新的每日更新。
    //   3. Request 閘門：等待已進入 HTTP/gRPC handler 的 request 返回。
    //   4. 資料閘門：等待 handler/cron 已建立的 backfill 與 DB 寫入完成。
    //   5. 常駐 task 閘門：最後停止只負責盤中輪詢、可以安全重建的價格 task。
    //
    // 這個順序很重要：如果先停掉底層價格 task，再等待上層 request，正在處理的 request
    // 可能突然失去相依服務；如果先等背景工作但仍接受新 request，active count 又永遠可能增加。
    // 直接 await 訊號 future，不再以 AtomicBool 每 100ms 輪詢，收到訊號後立即進入流程。
    let Running {
        shutdown_tx,
        mut sched,
        rpc_server,
        web_server,
        grpc_self_test,
    } = running;
    shutdown_signal().await?;
    tracing::info!("graceful shutdown begin");

    // 第 1 道閘門：watch 的值從 false 改成 true。
    // Axum/Tonic 收到後停止 accept 新連線；已經進入 handler 的 request 不會被硬切斷。
    let _ = shutdown_tx.send(true);

    // 第 2 道閘門：只停止「未來的排程觸發」。已開始的 cron future 不會被 scheduler
    // 強制 abort，而是由下方 BACKGROUND_OPERATIONS 負責等待，避免資料只寫一半。
    if let Err(why) = sched.shutdown().await {
        tracing::error!(error = %why, "scheduler shutdown failed");
    }

    // 自我測試不是必要的資料操作；若關機時仍未完成，主動取消以免延長服務停止時間。
    if !grpc_self_test.is_finished() {
        grpc_self_test.abort();
    }
    let _ = grpc_self_test.await;

    // 第 3 道閘門：HTTP/gRPC 各自排空既有 request。
    // 30 秒是 server 連線層的上限；它與下方資料工作五分鐘 timeout 是不同層級。
    let server_shutdown_timeout = tokio::time::Duration::from_secs(30);
    if let Some(rpc_server) = rpc_server {
        wait_for_server_shutdown("grpc", rpc_server, server_shutdown_timeout).await;
    }
    wait_for_server_shutdown("web", web_server, server_shutdown_timeout).await;

    // 第 4 道閘門：Server 已停止處理 request，scheduler 也不會再派發工作，因此 active
    // operation 只會減少、不會持續增加。先等待既有操作，避免提早停止其相依的價格服務。
    // 五分鐘到期後只記錄警告並繼續關機，避免單一故障 upstream 讓部署永遠無法停止。
    let background_timeout = tokio::time::Duration::from_secs(5 * 60);
    let operations = &core::shutdown::BACKGROUND_OPERATIONS;
    if !operations.wait_for_idle(background_timeout).await {
        tracing::warn!(
            active_operations = operations.active_count(),
            "graceful shutdown timed out while waiting for background operations"
        );
    }

    // 第 5 道閘門：資料操作完成後才停止盤中長生命週期 task。
    // stop_price_tasks 會喚醒正在 sleep/ticker 的 task，無須等待下一個 5 分鐘週期自然到期。
    app::event::trace::price_tasks::stop_price_tasks().await;

    tracing::info!("graceful shutdown done");
    println!("Server stopped gracefully");

    Ok(())
}
