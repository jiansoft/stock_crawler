use std::sync::OnceLock;

use crate::internal::{config, logging, util};

static DDNS_URL: OnceLock<String> = OnceLock::new();

/// 向ddns服務更新目前的IP
pub async fn update() {
    let url = DDNS_URL.get_or_init(|| {
        format!(
            "{}{}/{}/",
            config::SETTINGS.afraid.url,
            config::SETTINGS.afraid.path,
            config::SETTINGS.afraid.token,
        )
    });

    match util::http::get(url, None).await {
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
    use tokio_test;

    use super::*;

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
