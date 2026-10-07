//! `financial_statement` 查詢的整合測試（需要 PostgreSQL）。

use chrono::{Datelike, NaiveDate};
use std::time;

use rust_decimal_macros::dec;

use super::*;
use crate::core::declare::Quarter;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_annual() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_annual");

    let r = fetch_annual(2022).await;
    if let Ok(result) = r {
        tracing::debug!("{:?}", result);
    } else if let Err(err) = r {
        tracing::debug!("{:#?} ", err);
    }
    tracing::debug!("結束 fetch_annual");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_roe_is_zero() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_roe_is_zero");

    let r = fetch_roe_or_roa_equal_to_zero(Some(2023), Some(Quarter::Q3)).await;
    if let Ok(result) = r {
        dbg!(&result);
        tracing::debug!("{:?}", result);
    } else if let Err(err) = r {
        tracing::debug!("{:#?}", err);
    }
    tracing::debug!("結束 fetch_roe_is_zero");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_without_annual() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_without_annual");

    let current_date = NaiveDate::parse_from_str("2023-09-15", "%Y-%m-%d").unwrap();
    let r = fetch_without_annual(current_date.year()).await;
    match r {
        Ok(result) => {
            //dbg!(&result);
            tracing::debug!("{:#?}", result);
        }
        Err(err) => {
            tracing::debug!("{:#?}", err);
        }
    }
    tracing::debug!("結束 fetch_without_annual");
}

#[tokio::test]
#[ignore]
async fn test_fetch_cumulative_eps() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_cumulative_eps");

    let security_code = "2480";
    let year = 2023;
    let quarters = vec![Quarter::Q1, Quarter::Q2, Quarter::Q3];
    let eps = fetch_cumulative_eps(security_code, year, quarters).await;

    match eps {
        Ok(result) => {
            dbg!(&result);
            tracing::debug!("{:#?}", result);
            // 斷言結果
            assert_eq!(result, dec!(5.51));
        }
        Err(err) => {
            tracing::debug!("{:#?}", err);
        }
    }
    tracing::debug!("結束 fetch_cumulative_eps");
    tokio::time::sleep(time::Duration::from_secs(1)).await;
}
