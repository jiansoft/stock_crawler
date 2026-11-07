//! mod的測試；連外的測試標 #[ignore]。

use crate::infra::crawler::log_public_ip_visit_test;

use super::*;

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_visit() {
    log_public_ip_visit_test(visit).await;
}
