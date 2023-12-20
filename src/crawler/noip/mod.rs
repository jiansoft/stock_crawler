
use anyhow::{anyhow, Result};

use crate::{config, logging, util};

const HOST: &str = "dynupdate.no-ip.com";

/// 向ddns服務更新目前的IP
pub async fn visit(ip :&str) -> Result<()> {
    for hostname in &config::SETTINGS.noip.hostnames {
        let url =
            &format!(
                "https://{acount}:{pw}@{host}/nic/update?hostname={hostname}&myip={ip}",
                acount = config::SETTINGS.noip.username,
                pw = config::SETTINGS.noip.password,
                host = HOST,
                ip = ip,
                hostname = hostname
            );

        match util::http::get(url, None).await {
            Ok(t) => {
                dbg!(&t);
                if t.contains("good") {
                    logging::info_file_async(t);
                }

            }
            Err(why) => {
                return Err(anyhow!("Failed to noip.visit because {:?}", why));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::crawler::ipify;
    use super::*;

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
