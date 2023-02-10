use crate::internal::request_get;
use crate::{config, logging};
use concat_string::concat_string;

/// 向ddns服務更新目前的IP
pub async fn update() {
    let url = concat_string!(
        config::SETTINGS.afraid.url,
        config::SETTINGS.afraid.path,
        "?",
        config::DEFAULT.afraid.token
    );

    if let Some(t) = request_get(url).await {
        if t.contains("Updated") {
            logging::info_file_async(t);
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
