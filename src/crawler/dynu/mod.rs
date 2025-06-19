use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};

use crate::{config, logging, util};

static DDNS_URL: OnceLock<String> = OnceLock::new();

const HOST: &str = "api.dynu.com";

pub async fn visit(ip: &str) -> Result<()> {
    let url = DDNS_URL.get_or_init(|| {
        let mut hasher = Sha256::new();

        hasher.update(config::SETTINGS.dyny.password.as_bytes());

        let pw = hasher.finalize();

        format!(
            "https://{host}/nic/update?username={username}&password={pw}",
            host = HOST,
            username = config::SETTINGS.dyny.username,
            pw = hex::encode(pw)
        )
    });
    let url = format!("{url}&myip={ip}", url = url, ip = ip);

    match util::http::get(&url, None).await {
        Ok(t) => {
            if t.contains("good") {
                logging::info_file_async(t);
            }
        }
        Err(why) => {
            return Err(anyhow!("Failed to dynu.visit because {:?}", why));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crawler::ipify;

    #[tokio::test]
    async fn test_execute() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 visit".to_string());
        let ip_now = ipify::visit().await.unwrap();
        match visit(&ip_now).await {
            Ok(e) => {
                dbg!(e);
            }
            Err(why) => {
                logging::debug_file_async(format!("Failed to visit because {:?}", why));
            }
        }

        logging::debug_file_async("結束 visit".to_string());
    }
}
