/*#[macro_use]
extern crate rocket;*/

use std::{
    error::Error,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
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
///
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

async fn shutdown_signal_handler(received_signal: Arc<AtomicBool>) {
    if let Err(e) = signal::ctrl_c().await {
        eprintln!("Failed to listen for Ctrl+C signal: {}", e);
    }
    received_signal.store(true, Ordering::SeqCst);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
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
    cache::SHARE.load().await;

    let sched = JobScheduler::new().await?;
    scheduler::start(&sched).await?;
    rpc::server::start().await?;

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
