#[macro_use]
extern crate rocket;


use clokwerk::{AsyncScheduler, TimeUnits};
use rocket::tokio;
use rust_tutorial::{config, logging};
use std::time::Duration;

#[get("/")]
fn index() -> &'static str {
    "Hello, world!"
}

#[launch]
fn rocket() -> _ {

    init();
    rocket::build().mount("/", routes![index])
}

fn init() {
    dotenv::dotenv().ok();

    for _ in 0..10 {
        logging::info_file_async(format!("DEFAULT {:?}", config::DEFAULT.afraid));
        logging::info_file_async(format!("SETTINGS {:?}", config::SETTINGS.afraid));
    }


    let mut scheduler = AsyncScheduler::new();
    scheduler.every(60.seconds()).run(|| async {
        let url = format!(
            "https://freedns.afraid.org/dynamic/update.php?{}",
            config::DEFAULT.afraid.token
        );

        match reqwest::get(url).await {
            Ok(res) => match res.text().await {
                Ok(t) => {
                    logging::info_file_async(t);
                }
                Err(why) => {
                    logging::error_file_async(format!("{:?}", why));
                }
            },
            Err(why) => {
                logging::error_file_async(format!("{:?}", why));
            }
        }
    });

    tokio::spawn(async move {
        loop {
            scheduler.run_pending().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
}
/*
cargo +nightly udeps --all-targets
rustup default stable-x86_64-pc-windows-gnu
rustup default stable-x86_64-pc-windows-msvc
*/
/*#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    //simplelog::WriteLogger::init(simplelog::LevelFilter::Info, simplelog::Config::default(), File::create("app.log").unwrap()).unwrap();
    //SimpleLogger::new().with_local_timestamps().init().unwrap();

    //log::info!("This is an example message.");
    // Create a new scheduler
    let mut scheduler = AsyncScheduler::new();
    // or a scheduler with a given timezone
    //let mut scheduler = Scheduler::with_tz(chrono::Utc);

    // Add some tasks to it
    scheduler
        .every(60.seconds())
        .run(|| {
            async {
                let resp = reqwest::get("https://freedns.afraid.org/dynamic/update.php?N1RpRlJudzdJWGVGelRURGJkOXdRMTlrOjIxMDA3MDM2")
                    .await.expect("REASON").text().await;

                println!("{} {}", DelayedFormat::to_string(&Local::now().format("%Y-%m-%d %H:%M:%S.%3f")), resp.unwrap());
            }
        });

    tokio::spawn(async move {
        loop {
            scheduler.run_pending().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    // Or run it in a background thread
    //let thread_handle = scheduler.watch_thread(Duration::from_millis(100));

    /* let resp = reqwest::get("https://freedns.afraid.org/dynamic/update.php?N1RpRlJudzdJWGVGelRURGJkOXdRMTlrOjIxMDA3MDM2")
        .await?.text().await?
        ;
    println!("{:#?}", resp);*/

    loop {
        thread::sleep(Duration::from_secs(100));
    }
    //thread_handle.stop();
    // Ok(())
}
*/
