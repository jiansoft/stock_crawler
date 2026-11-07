//! payout_ratio的測試；連外的測試標 #[ignore]。

use crate::infra::cache::SHARE;

use super::*;

/// 手動重算全部盈餘分配率（會寫入 `.env` 指向的資料庫）。
/// 失敗必須讓測試失敗：只記 debug 日誌的話，執行者看到 ok 會以為已經寫入。
#[tokio::test]
#[ignore]
async fn test_execute() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    execute().await.expect("payout_ratio::execute 失敗");
}
