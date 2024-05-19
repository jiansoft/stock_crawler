use anyhow::{anyhow, Result};

use crate::{
    crawler::{self, share},
    declare,
    logging,
    nosql,
    cache::SHARE
};

pub async fn refresh() -> Result<()> {
    let ip_now = share::get_public_ip().await?;

    if ip_now.is_empty() {
        return Err(anyhow!(
            "The IP addresses of ipify and seeip responses are empty."
        ));
    }

    let ddns_key = format!("MyPublicIP:{ip}", ip = ip_now);

    if let Ok(exist) = nosql::redis::CLIENT.contains_key(&ddns_key).await {
        if exist {
            return Ok(());
        }
    }
    
    SHARE.set_current_ip(ip_now.clone());
    
    update_ddns_services(&ip_now).await;
    
    nosql::redis::CLIENT
        .set(ddns_key, ip_now, declare::ONE_DAYS_IN_SECONDS)
        .await?;

    Ok(())
}

async fn update_ddns_services(ip: &str) {
    let afraid = crawler::afraid::visit();
    let dynu = crawler::dynu::visit(ip);
    let noip = crawler::noip::visit(ip);
    let (res_dynu, res_afraid, res_noip) = tokio::join!(dynu, afraid, noip);

    log_error("dynu", res_dynu).await;
    log_error("afraid", res_afraid).await;
    log_error("noip", res_noip).await;
}

async fn log_error(service_name: &str, result: Result<()>) {
    if let Err(why) = result {
        logging::error_file_async(format!(
            "Failed to {}::visit() because {:#?}",
            service_name, why
        ));
    }
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
