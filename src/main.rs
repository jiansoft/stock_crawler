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
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use tokio::signal;
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal as unix_signal};
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
#[unsafe(link_section = ".init_array")]
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
    // 先載入本機環境設定，讓後續 logging 與 SETTINGS 都能吃到 .env 覆蓋值。
    dotenv::dotenv().ok();

    // ── 日誌系統初始化 ────────────────────────────────────────────────────────
    //
    // 使用 tracing + tracing-subscriber 建立兩層 subscriber：
    //
    // Layer 1 — fmt（標準輸出）
    //   由 RUST_LOG 環境變數控制要不要輸出，以及輸出哪個等級。
    //   預設不設定 RUST_LOG 時完全靜音，不影響生產環境。
    //   開發除錯時執行：RUST_LOG=info cargo run
    //
    // Layer 2 — FileLogLayer（輪轉日誌檔，always-on）
    //   實作在 core::logging::FileLogLayer，不受 RUST_LOG 控制。
    //   每個 tracing 事件都會寫入 log/YYYY-MM-DD_default_{level}.log。
    //   底層仍使用原有的非同步輪轉機制（core::logging::LOGGER）。
    //
    // 事件流向：
    //   tracing::info!("...")
    //     ├─ FileLogLayer → LOGGER → 輪轉日誌檔（+ Seq，若有設定）
    //     └─ fmt layer    → stdout（僅 RUST_LOG 啟用時）
    //
    // 注意：subscriber 必須在任何 tracing::*! 呼叫之前完成初始化，
    //       否則初始化前的事件會被靜默丟棄。
    {
        use tracing_subscriber::Layer;
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_filter(tracing_subscriber::EnvFilter::from_default_env()),
            )
            .with(core::logging::FileLogLayer)
            .init();
    }

    // Seq 是選用的結構化日誌收集器（設定於 app.json logging.seq）。
    // 未設定時此函式直接返回，不影響檔案日誌正常運作。
    core::logging::init_seq(
        &core::config::SETTINGS.logging.seq.server_url,
        &core::config::SETTINGS.logging.seq.api_key,
    )
    .await;

    let startup_timer = Instant::now();
    let received_signal = Arc::new(AtomicBool::new(false));
    let allocator_tuning = core::util::diagnostics::tune_allocator_for_long_running_process();

    #[cfg(all(target_os = "linux", target_env = "musl"))]
    tracing::info!("{}", "allocator profile: mimalloc (musl target), purge_delay=0, purge_decommits=1, thp=0, large_os_pages=0, arena_eager_commit=0"
            .to_string(),);

    #[cfg(not(all(target_os = "linux", target_env = "musl")))]
    tracing::info!("allocator profile: system allocator");

    if allocator_tuning.applied {
        tracing::info!("allocator tuning applied arena_max={} trim_threshold={}",
            allocator_tuning.arena_max_applied, allocator_tuning.trim_threshold_applied,);
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

    // 在進入快取載入與背景服務前，先對資料庫進行連線檢查（Fail Fast）
    tracing::info!("startup database check: ping database");
    if let Err(e) = infra::database::ping().await {
        let err_msg = format!("Failed to connect to database: {:?}", e);
        tracing::error!("{}", &err_msg);
        interfaces::bot::telegram::send_alert("資料庫連線失敗（主機啟動異常）", &err_msg).await;
        return Err(err_msg.into());
    }
    tracing::info!("startup database check: database is online");

    tracing::info!("{}", "startup phase begin: crate::infra::cache::SHARE.load".to_string(),);
    let cache_load_timer = Instant::now();
    infra::cache::SHARE.load().await;
    tracing::info!("startup phase done: crate::infra::cache::SHARE.load elapsed={:?}",
        cache_load_timer.elapsed());

    tracing::info!("startup phase begin: JobScheduler::new");
    let scheduler_new_timer = Instant::now();
    let sched = JobScheduler::new().await?;
    tracing::info!("startup phase done: JobScheduler::new elapsed={:?}",
        scheduler_new_timer.elapsed());

    tracing::info!("startup phase begin: scheduler::start");
    let scheduler_start_timer = Instant::now();
    app::scheduler::start(&sched).await?;
    tracing::info!("startup phase done: scheduler::start elapsed={:?}",
        scheduler_start_timer.elapsed());

    tracing::info!("startup phase begin: rpc::server::start");
    let rpc_start_timer = Instant::now();
    if let Err(why) = interfaces::rpc::server::start().await {
        let err_msg = format!("gRPC server failed to start: {:?}", why);
        tracing::error!("{}", &err_msg);
        interfaces::bot::telegram::send_alert("gRPC 伺服器啟動失敗", &err_msg).await;
        return Err(why.into());
    }
    tracing::info!("startup phase done: rpc::server::start elapsed={:?}",
        rpc_start_timer.elapsed());

    tracing::info!("startup phase begin: web::start");
    let web_start_timer = Instant::now();
    if let Err(why) = interfaces::web::start().await {
        let err_msg = format!("Web server failed to start: {:?}", why);
        tracing::error!("{}", &err_msg);
        interfaces::bot::telegram::send_alert("Web 伺服器啟動失敗", &err_msg).await;
        return Err(why.into());
    }
    tracing::info!("startup phase done: web::start elapsed={:?}",
        web_start_timer.elapsed());
    tracing::info!("startup phase done: main init total elapsed={:?}",
        startup_timer.elapsed());

    // 啟動後延遲測試 gRPC 連線
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        if let Err(why) = interfaces::rpc::client::test_client::run_test().await {
            let err_msg = format!("gRPC 自我測試失敗: {:?}", why);
            tracing::error!("{}", &err_msg);
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
            tracing::error!("{}", &err_msg);
            interfaces::bot::telegram::send_alert("Redis 快取連線失敗", &err_msg).await;
        }
    }

    while !received_signal.load(Ordering::SeqCst) {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    println!("Server stopped: {:?}", received_signal);

    Ok(())
}
