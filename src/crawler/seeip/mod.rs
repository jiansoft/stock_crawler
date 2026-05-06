use std::sync::OnceLock;

use anyhow::Result;

use crate::crawler;

static DDNS_URL: OnceLock<String> = OnceLock::new();

const HOST: &str = "ipv4.seeip.org";

/// 取得目前的IP
pub async fn visit() -> Result<String> {
    crawler::get_public_ip_text(&DDNS_URL, HOST, "", false).await
}

#[cfg(test)]
mod tests {
    use crate::crawler::log_public_ip_visit_test;

    use super::*;

    #[tokio::test]
    async fn test_visit() {
        log_public_ip_visit_test(visit).await;
    }
}
