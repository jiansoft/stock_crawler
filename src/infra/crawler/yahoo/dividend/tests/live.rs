//! 連線 Yahoo 正式站的手動測試（`#[ignore]`）。

use super::*;

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 visit");

    match visit("2357").await {
        Ok(e) => {
            dbg!(&e);
        }
        Err(why) => {
            dbg!(&why);
        }
    }

    tracing::debug!("結束 visit");
}
