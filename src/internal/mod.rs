use crate::logging;
use once_cell::sync::Lazy;
use reqwest::{Client, IntoUrl};

mod crawler;
pub mod database;
mod free_dns;
pub mod scheduler;
pub mod cache_share;

static CLIENT: Lazy<Client> = Lazy::new(Default::default);

///
pub async fn request_get<T: IntoUrl>(url: T) -> Option<String> {
    let res = CLIENT.get(url).send().await;
    match res {
        Ok(res) => {
            return match res.text().await {
                Ok(t) => Some(t),
                Err(why) => {
                    logging::error_file_async(format!("{:?}", why));
                    None
                }
            }
        }
        Err(why) => {
            logging::error_file_async(format!("{:?}", why));
            None
        }
    }
}
