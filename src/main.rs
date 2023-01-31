#[macro_use]
extern crate rocket;

use rust_tutorial::{config, internal::scheduler, logging};

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
