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
    let noip = crawler::noip::visit(&ip_now);
    let (res_dynu, res_afraid,res_noip) = tokio::join!(dynu, afraid,noip);

    if let Err(why) = res_dynu {
        logging::error_file_async(format!("Failed to dynu::visit() because {:#?}", why));
    }

    if let Err(why) = res_afraid {
        logging::error_file_async(format!("Failed to afraid::visit() because {:#?}", why));
    }

    if let Err(why) = res_noip {
        logging::error_file_async(format!("Failed to noip::visit() because {:#?}", why));
    }

    nosql::redis::CLIENT
        .set(ddns_key, ip_now, declare::ONE_DAYS_IN_SECONDS)
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
