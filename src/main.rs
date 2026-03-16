//! `stock_crawler` 可執行程式入口。
//!
//! 主要職責：
//! - 載入環境與主快取。
//! - 啟動排程與 gRPC 服務。
//! - 註冊結束訊號，讓行程可平順停止。
//! - 啟動後做一次 gRPC 自我連線驗證。

/*#[macro_use]
extern crate rocket;*/

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

/// 數據回補
pub mod backfill;
/// 聊天機器人
pub mod bot;
/// 數據快取
pub mod cache;
/// 計算類
pub mod calculation;
/// 設定檔
pub mod config;
/// 抓取數據類
pub mod crawler;
/// 資料庫操作
pub mod database;
/// 定義結構、enum等
pub mod declare;
/// 事件
pub mod event;
/// 日誌
pub mod logging;
/// nosql
pub mod nosql;
/// RPC 模組
pub mod rpc;
/// 工作排程
pub mod scheduler;
/// 工具類
pub mod util;

/*#[get("/")]
fn index() -> &'static str {
    "Hello, world!"
}

#[get("/world")]
fn world() -> &'static str {
    "Hello, world!"
}

#[rocket::main]
async fn main() -> Result<(), rocket::Error> {
    dotenv::dotenv().ok();
    cache_share::CACHE_SHARE.load().await;
    scheduler::start().await;

    let _rocket = rocket::build()
        .mount("/hello", routes![world])
        .mount("/", routes![index])
        .launch()
        .await?;

    Ok(())
}
*/

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
    logging::info_file_async("startup phase begin: cache::SHARE.load".to_string());
    let cache_load_timer = Instant::now();
    cache::SHARE.load().await;
    logging::info_file_async(format!(
        "startup phase done: cache::SHARE.load elapsed={:?}",
        cache_load_timer.elapsed()
    ));
    
    logging::info_file_async("startup phase begin: JobScheduler::new".to_string());
    let scheduler_new_timer = Instant::now();
    let sched = JobScheduler::new().await?;
    logging::info_file_async(format!(
        "startup phase done: JobScheduler::new elapsed={:?}",
        scheduler_new_timer.elapsed()
    ));

    logging::info_file_async("startup phase begin: scheduler::start".to_string());
    let scheduler_start_timer = Instant::now();
    scheduler::start(&sched).await?;
    logging::info_file_async(format!(
        "startup phase done: scheduler::start elapsed={:?}",
        scheduler_start_timer.elapsed()
    ));

    logging::info_file_async("startup phase begin: rpc::server::start".to_string());
    let rpc_start_timer = Instant::now();
    rpc::server::start().await?;
    logging::info_file_async(format!(
        "startup phase done: rpc::server::start elapsed={:?}",
        rpc_start_timer.elapsed()
    ));
    logging::info_file_async(format!(
        "startup phase done: main init total elapsed={:?}",
        startup_timer.elapsed()
    ));

    // 啟動後延遲測試 gRPC 連線
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        if let Err(why) = rpc::client::test_client::run_test().await {
            logging::error_file_async(format!("gRPC 自我測試失敗: {}", why));
        }
    });

    let pong = nosql::redis::CLIENT.ping().await;
    if let Ok(pong) = pong {
        println!("pong: {}", pong);
    }

    while !received_signal.load(Ordering::SeqCst) {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    println!("Server stopped: {:?}", received_signal);

    Ok(())
}

/*
要計算價格下降的百分比，可以使用以下的公式：
百分比變動=(新值−舊值) / 舊值 × 100%
*/
