use std::sync::OnceLock;

use anyhow::Result;

use crate::infra::crawler;

static DDNS_URL: OnceLock<String> = OnceLock::new();

const HOST: &str = "api.ipify.org";

/// 取得目前的IP
pub async fn visit() -> Result<String> {
    Ok(crawler::get_public_ip_text(&DDNS_URL, HOST, "", false).await?)
}

#[cfg(test)]
mod tests {
    use crate::infra::crawler::log_public_ip_visit_test;

    use super::*;

    #[tokio::test]
    #[ignore = "live test：連線真實外部網站，需要時手動執行"]
    async fn test_visit() {
        log_public_ip_visit_test(visit).await;
    }
}
