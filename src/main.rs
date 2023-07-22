/*#[macro_use]
extern crate rocket;*/

use std::{
    error::Error,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc
    }
};

use tokio::signal;
#[cfg(unix)]
use tokio::signal::unix::{signal as unix_signal, SignalKind};

use crate::internal::{cache, nosql, scheduler};

pub mod internal;

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
    scheduler::start().await;
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
