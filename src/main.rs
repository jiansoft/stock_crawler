#[macro_use]
extern crate rocket;

use rust_tutorial::{
    config, internal::crawler::taiwan_capitalization_weighted_stock_index, internal::scheduler,
    logging,
};

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
    for _ in 0..10 {
        // let d = format!("DEFAULT {:?}", config::DEFAULT.afraid);
        let s = format!("SETTINGS {:?}", config::SETTINGS.afraid);
        logging::info_file_async(format!("DEFAULT {:?}", config::DEFAULT.afraid));
        logging::info_file_async(s);
    }

    taiwan_capitalization_weighted_stock_index::visit().await;
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
