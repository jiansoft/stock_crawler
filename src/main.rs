//! `stock_crawler` 可執行程式入口。
//!
//! 主要職責：
//! - 載入環境與主快取。
//! - 啟動排程與 gRPC 服務。
//! - 註冊結束訊號，讓行程可平順停止。
//! - 啟動後做一次 gRPC 自我連線驗證。

#[cfg(all(target_os = "linux", target_env = "musl"))]
use mimalloc::MiMalloc;
#[cfg(all(target_os = "linux", target_env = "musl"))]
use std::ffi::c_char;
use std::{
    error::Error,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

use tokio::signal;
#[cfg(unix)]
use tokio::signal::unix::{signal as unix_signal, SignalKind};
use tokio_cron_scheduler::JobScheduler;

#[cfg(all(target_os = "linux", target_env = "musl"))]
#[global_allocator]
static GLOBAL_ALLOCATOR: MiMalloc = MiMalloc;

/// 在 `main` 之前為 musl + mimalloc 設定較積極的回收策略。
///
/// 這些選項來自 mimalloc 官方環境參數：
/// - `MIMALLOC_PURGE_DELAY=0`：頁面一旦空出就立即 purge。
/// - `MIMALLOC_PURGE_DECOMMITS=1`：purge 時用 decommit / `MADV_DONTNEED`，讓 RSS 較快下降。
/// - `MIMALLOC_ALLOW_THP=0`：避免 THP 讓 purge 顆粒變粗，影響回收速度。
/// - `MIMALLOC_ALLOW_LARGE_OS_PAGES=0`：避免不可 purge 的 large OS pages。
/// - `MIMALLOC_ARENA_EAGER_COMMIT=0`：避免過早 commit 大 arena。
#[cfg(all(target_os = "linux", target_env = "musl"))]
#[used]
#[cfg_attr(
    all(target_os = "linux", target_env = "musl"),
    link_section = ".init_array"
)]
static INIT_MIMALLOC_ENV: extern "C" fn() = init_mimalloc_env;

#[cfg(all(target_os = "linux", target_env = "musl"))]
extern "C" fn init_mimalloc_env() {
    unsafe {
        setenv(
            b"MIMALLOC_PURGE_DELAY\0".as_ptr().cast::<c_char>(),
            b"0\0".as_ptr().cast::<c_char>(),
            0,
        );
        setenv(
            b"MIMALLOC_PURGE_DECOMMITS\0".as_ptr().cast::<c_char>(),
            b"1\0".as_ptr().cast::<c_char>(),
            0,
        );
        setenv(
            b"MIMALLOC_ALLOW_THP\0".as_ptr().cast::<c_char>(),
            b"0\0".as_ptr().cast::<c_char>(),
            0,
        );
        setenv(
            b"MIMALLOC_ALLOW_LARGE_OS_PAGES\0".as_ptr().cast::<c_char>(),
            b"0\0".as_ptr().cast::<c_char>(),
            0,
        );
        setenv(
            b"MIMALLOC_ARENA_EAGER_COMMIT\0".as_ptr().cast::<c_char>(),
            b"0\0".as_ptr().cast::<c_char>(),
            0,
        );
    }
}

#[cfg(all(target_os = "linux", target_env = "musl"))]
unsafe extern "C" {
    fn setenv(name: *const c_char, value: *const c_char, overwrite: i32) -> i32;
}

pub mod app;
pub mod core;
pub mod domain;
pub mod infra;
pub mod interfaces;

/// 在 Unix 平台監聽 `SIGINT` / `SIGTERM`，並通知主迴圈結束。
#[cfg(unix)]
async fn unix_signal_handler(received_signal: Arc<AtomicBool>) -> Result<(), Box<dyn Error>> {
    let mut sigint = unix_signal(SignalKind::interrupt())?;
    let mut sigterm = unix_signal(SignalKind::terminate())?;

    tokio::select! {
        _ = sigint.recv() => {}
        _ = sigterm.recv() => {}
    }

    received_signal.store(true, Ordering::SeqCst);

    Ok(())
}

/// 監聽跨平台 `Ctrl+C` 訊號，並通知主迴圈結束。
async fn shutdown_signal_handler(received_signal: Arc<AtomicBool>) {
    if let Err(e) = signal::ctrl_c().await {
        eprintln!("Failed to listen for Ctrl+C signal: {}", e);
    }
    received_signal.store(true, Ordering::SeqCst);
}

/// 啟動 `stock_crawler` 主流程。
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let startup_timer = Instant::now();
    let received_signal = Arc::new(AtomicBool::new(false));
    let allocator_tuning = core::util::diagnostics::tune_allocator_for_long_running_process();

    #[cfg(all(target_os = "linux", target_env = "musl"))]
    core::logging::info_file_async(
        "allocator profile: mimalloc (musl target), purge_delay=0, purge_decommits=1, thp=0, large_os_pages=0, arena_eager_commit=0"
            .to_string(),
    );

    #[cfg(not(all(target_os = "linux", target_env = "musl")))]
    core::logging::info_file_async("allocator profile: system allocator".to_string());

    if allocator_tuning.applied {
        core::logging::info_file_async(format!(
            "allocator tuning applied arena_max={} trim_threshold={}",
            allocator_tuning.arena_max_applied, allocator_tuning.trim_threshold_applied,
        ));
    }

    tokio::spawn(shutdown_signal_handler(received_signal.clone()));

    #[cfg(unix)]
    tokio::spawn({
        let received_signal = Arc::clone(&received_signal);
        async move {
            if let Err(e) = unix_signal_handler(received_signal).await {
                eprintln!("Error handling unix signals: {}", e);
            }
        }
    });

    dotenv::dotenv().ok();

    // 在進入快取載入與背景服務前，先對資料庫進行連線檢查（Fail Fast）
    core::logging::info_file_async("startup database check: ping database".to_string());
    if let Err(e) = infra::database::ping().await {
        let err_msg = format!("Failed to connect to database: {:?}", e);
        core::logging::error_file_async(&err_msg);
        interfaces::bot::telegram::send_alert("資料庫連線失敗（主機啟動異常）", &err_msg).await;
        return Err(err_msg.into());
    }
    core::logging::info_file_async("startup database check: database is online".to_string());

    core::logging::info_file_async(
        "startup phase begin: crate::infra::cache::SHARE.load".to_string(),
    );
    let cache_load_timer = Instant::now();
    infra::cache::SHARE.load().await;
    core::logging::info_file_async(format!(
        "startup phase done: crate::infra::cache::SHARE.load elapsed={:?}",
        cache_load_timer.elapsed()
    ));

    core::logging::info_file_async("startup phase begin: JobScheduler::new".to_string());
    let scheduler_new_timer = Instant::now();
    let sched = JobScheduler::new().await?;
    core::logging::info_file_async(format!(
        "startup phase done: JobScheduler::new elapsed={:?}",
        scheduler_new_timer.elapsed()
    ));

    core::logging::info_file_async("startup phase begin: scheduler::start".to_string());
    let scheduler_start_timer = Instant::now();
    app::scheduler::start(&sched).await?;
    core::logging::info_file_async(format!(
        "startup phase done: scheduler::start elapsed={:?}",
        scheduler_start_timer.elapsed()
    ));

    core::logging::info_file_async("startup phase begin: rpc::server::start".to_string());
    let rpc_start_timer = Instant::now();
    if let Err(why) = interfaces::rpc::server::start().await {
        let err_msg = format!("gRPC server failed to start: {:?}", why);
        core::logging::error_file_async(&err_msg);
        interfaces::bot::telegram::send_alert("gRPC 伺服器啟動失敗", &err_msg).await;
        return Err(why.into());
    }
    core::logging::info_file_async(format!(
        "startup phase done: rpc::server::start elapsed={:?}",
        rpc_start_timer.elapsed()
    ));

    core::logging::info_file_async("startup phase begin: web::start".to_string());
    let web_start_timer = Instant::now();
    if let Err(why) = interfaces::web::start().await {
        let err_msg = format!("Web server failed to start: {:?}", why);
        core::logging::error_file_async(&err_msg);
        interfaces::bot::telegram::send_alert("Web 伺服器啟動失敗", &err_msg).await;
        return Err(why.into());
    }
    core::logging::info_file_async(format!(
        "startup phase done: web::start elapsed={:?}",
        web_start_timer.elapsed()
    ));
    core::logging::info_file_async(format!(
        "startup phase done: main init total elapsed={:?}",
        startup_timer.elapsed()
    ));

    // 啟動後延遲測試 gRPC 連線
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        if let Err(why) = interfaces::rpc::client::test_client::run_test().await {
            let err_msg = format!("gRPC 自我測試失敗: {:?}", why);
            core::logging::error_file_async(&err_msg);
            interfaces::bot::telegram::send_alert("gRPC 自我測試失敗", &err_msg).await;
        }
    });

    let pong = crate::infra::nosql::redis::CLIENT.ping().await;
    match pong {
        Ok(pong_val) => {
            println!("pong: {}", pong_val);
        }
        Err(why) => {
            let err_msg = format!("Redis ping failed at startup: {:?}", why);
            core::logging::error_file_async(&err_msg);
            interfaces::bot::telegram::send_alert("Redis 快取連線失敗", &err_msg).await;
        }
    }

    while !received_signal.load(Ordering::SeqCst) {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    println!("Server stopped: {:?}", received_signal);

    Ok(())
}
