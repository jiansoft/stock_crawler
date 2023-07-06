use crate::internal::{config, logging, util};
use std::sync::OnceLock;

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

    logging::debug_file_async(format!("visit url:{}", url.as_str(),));
    match util::http::get(url, None).await {
        Ok(t) => {
            // if t.contains("Updated") {
            logging::debug_file_async(t);
            // }
        }
        Err(why) => {
            logging::error_file_async(format!("Failed to request_get because {:?}", why));
        }
    }
    logging::debug_file_async(format!("visit url:{} finish", url.as_str(),));
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
