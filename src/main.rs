#[macro_use]
extern crate rocket;

use rust_tutorial::{internal::cache_share, internal::scheduler};

#[get("/")]
fn index() -> &'static str {
    "Hello, world!"
}

#[get("/world")]
fn world() -> &'static str {
    "Hello, world!"
}

async fn setup() {
    dotenv::dotenv().ok();

    cache_share::CACHE_SHARE.load().await;
    scheduler::start().await;
}

#[rocket::main]
async fn main() -> Result<(), rocket::Error> {
    setup().await;
    let _rocket = rocket::build()
        .mount("/hello", routes![world])
        .mount("/", routes![index])
        .launch()
        .await?;

    Ok(())
}
