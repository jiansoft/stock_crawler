/*#[macro_use]
extern crate rocket;*/

pub mod config;
pub mod internal;
pub mod logging;

use crate::internal::{cache_share, scheduler};
use std::{
    error::Error,
    sync::atomic::{AtomicBool, Ordering},
    sync::Arc,
};
use tokio::signal;

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // 創建 AtomicBool 以跟蹤是否收到信號
    let received_signal = Arc::new(AtomicBool::new(false));
    let received_signal_clone = Arc::clone(&received_signal);

    // 捕獲 Ctrl+C 信號
    tokio::spawn(async move {
        signal::ctrl_c()
            .await
            .expect("Failed to listen for Ctrl+C signal");
        received_signal_clone.store(true, Ordering::SeqCst);
    });

    // 捕獲 SIGINT 和 SIGTERM 信號（僅適用於 Unix）
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let received_signal_clone = Arc::clone(&received_signal);
        tokio::spawn(async move {
            let mut sigint =
                signal(SignalKind::interrupt()).expect("Failed to register SIGINT signal");
            let mut sigterm =
                signal(SignalKind::terminate()).expect("Failed to register SIGTERM signal");

            tokio::select! {
                _ = sigint.recv() => {}
                _ = sigterm.recv() => {}
            }

            received_signal_clone.store(true, Ordering::SeqCst);
        });
    }

    dotenv::dotenv().ok();
    cache_share::CACHE_SHARE.load().await;
    scheduler::start().await;

    // 等待收到信號
    while !received_signal.load(Ordering::SeqCst) {
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
    }

    println!("Server stopped: {:?}", received_signal);

    Ok(())
}
