use std::sync::OnceLock;

use anyhow::{anyhow, Result};

use crate::{config, logging, util};

static DDNS_URL: OnceLock<String> = OnceLock::new();

/// 向ddns服務更新目前的IP
pub async fn visit() -> Result<()> {
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
            return Err(anyhow!("Failed to afraid.visit because {:?}", why));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use tokio_test;

    use super::*;

    macro_rules! aw {
        ($e:expr) => {
            let _ = tokio_test::block_on($e);
        };
    }

    #[test]
    #[ignore]
    fn test_visit() {
        dotenv::dotenv().ok();
        aw!(visit());
    }
}
