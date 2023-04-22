use crate::{internal::{
    config,
    util,
    logging
}};
use concat_string::concat_string;

/// 向ddns服務更新目前的IP
pub async fn update() {
    let url = concat_string!(
        config::SETTINGS.afraid.url,
        config::SETTINGS.afraid.path,
        "?",
        config::SETTINGS.afraid.token
    );

    match util::http::request_get(&url).await {
        Ok(t) => {
            if t.contains("Updated") {
                logging::info_file_async(t);
            }
        }
        Err(why) => {
            logging::error_file_async(format!("Failed to request_get because {:?}", why));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_test;

    macro_rules! aw {
        ($e:expr) => {
            tokio_test::block_on($e)
        };
    }

    #[test]
    fn test_update() {
        dotenv::dotenv().ok();
        aw!(update());
    }
}
