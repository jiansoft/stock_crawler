use anyhow::{anyhow, Result};

use crate::{
    crawler::{self},
    declare, logging, nosql,
};

pub async fn refresh() -> Result<()> {
    let ip_now = crawler::ipify::visit().await?;

    if ip_now.is_empty() {
        return Err(anyhow!("The IP address of the ipify response is empty"));
    }

    let ddns_key = format!("MyPublicIP:{ip}", ip = ip_now);
    if let Ok(exist) = nosql::redis::CLIENT.contains_key(&ddns_key).await {
        if exist {
            return Ok(());
        }
    }

    let afraid = crawler::afraid::visit();
    let dynu = crawler::dynu::visit(&ip_now);
    let (res_dynu, res_afraid) = tokio::join!(dynu, afraid);

    if let Err(why) = res_dynu {
        logging::error_file_async(format!("Failed to dynu::execute() because {:#?}", why));
    }

    if let Err(why) = res_afraid {
        logging::error_file_async(format!("Failed to afraid::execute() because {:#?}", why));
    }

    nosql::redis::CLIENT
        .set(ddns_key, ip_now, declare::THREE_DAYS_IN_SECONDS)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_execute() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 refresh".to_string());

        match refresh().await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to refresh because {:?}", why));
            }
        }

        logging::debug_file_async("結束 refresh".to_string());
    }
}
