#[macro_use]
extern crate rocket;

pub mod config;
pub mod internal;
pub mod logging;

use crate::internal::{cache_share, scheduler};

#[get("/")]
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
