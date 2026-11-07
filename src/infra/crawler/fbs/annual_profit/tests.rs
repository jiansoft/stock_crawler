//! annual_profit的測試；連外的測試標 #[ignore]。

use super::*;

/// 網址一旦改錯就會整個來源失效，這裡把格式鎖定住。
#[test]
fn build_url_points_to_annual_profit_page() {
    assert_eq!(
        build_url("2330"),
        "https://fubon-ebrokerdj.fbs.com.tw/z/zc/zcdj_2330.djhtm"
    );
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_visit() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 visit");

    match visit("2330").await {
        Ok(e) => {
            dbg!(&e);
            tracing::debug!("fbs : {:#?}", e);
        }
        Err(why) => {
            tracing::debug!("Failed to visit because {:?}", why);
        }
    }

    tracing::debug!("結束 visit");
}
