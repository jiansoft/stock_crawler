//! 股利發放日資訊查詢的測試。

use core::result::Result::Ok;

use chrono::{Local, TimeZone};

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_stocks_with_payable_on_date() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_stocks_with_payable_on_date");

    let ex_date = Local.with_ymd_and_hms(2023, 8, 25, 0, 0, 0).unwrap();
    let d = ex_date.date_naive();
    match fetch(d).await {
        Ok(cd) => {
            dbg!(&cd);
            tracing::debug!("stock: {:?}", cd);
        }
        Err(why) => {
            tracing::debug!(
                "Failed to fetch_stocks_with_payable_on_date because {:?}",
                why
            );
        }
    }

    tracing::debug!("結束 fetch_stocks_with_payable_on_date");
}
