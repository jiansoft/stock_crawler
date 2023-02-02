#[macro_use]
extern crate rocket;

use rust_tutorial::internal::cache_share;
use rust_tutorial::{internal::scheduler, logging};

#[get("/")]
fn index() -> &'static str {
    "Hello, world!"
}

#[get("/world")]
fn world() -> &'static str {
    "Hello, world!"
}

async fn init() {
    dotenv::dotenv().ok();
    /*for _ in 0..10 {
        // let d = format!("DEFAULT {:?}", config::DEFAULT.afraid);
        let s = format!("SETTINGS.afraid {:?}", config::SETTINGS.afraid);
        logging::info_file_async(format!("DEFAULT.afraid {:?}", config::DEFAULT.afraid));
        logging::info_file_async(format!(
            "DEFAULT.postgresql {:?}",
            config::DEFAULT.postgresql
        ));
        logging::info_file_async(s);
        logging::info_file_async(format!(
            "SETTINGS.postgresql {:?}",
            config::SETTINGS.postgresql
        ));
    }*/
    //let pi = Decimal::from_f64(3141.3694).unwrap();
    //logging::info_file_async(pi.to_string());

    if cache_share::CACHE_SHARE.indices.read().unwrap().contains_key("2023-02-01_TAIEX") {
        logging::info_file_async(format!("key 存在"));
        for e in cache_share::CACHE_SHARE.indices.read().unwrap().iter() {
            logging::info_file_async(format!(
                "main.indices e.date {:?} e.index {:?}",
                e.1.date, e.1.index
            ));
        }
    }

    scheduler::start().await;
}

#[rocket::main]
async fn main() -> Result<(), rocket::Error> {
    init().await;
    let _rocket = rocket::build()
        .mount("/hello", routes![world])
        .mount("/", routes![index])
        .launch()
        .await?;

    Ok(())
}
