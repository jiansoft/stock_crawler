//! 手動回補：ETF 專區。
//!
//! - `test_backfill_etf_profiles`：逐檔抓 Yahoo ETF 頁，寫入基本資料、分類與持股並重建快照
//!   （與週六排程相同，約 15 分鐘）；已有資料覆蓋，重跑安全。
//! - `test_backfill_etf_daily`：抓證交所 MIS 預估淨值、補推斷分類、重算含息績效並重建快照
//!   （與週一～五 17:30 排程相同）；週末不執行。
//! - `test_refresh_etf_snapshot`：只重建清單頁快照。

use crate::app::backfill::etf_zone;

/// 抓 Yahoo ETF 頁的基本資料、分類與持股。
#[tokio::test]
#[ignore]
async fn test_backfill_etf_profiles() {
    dotenvy::dotenv().ok();
    etf_zone::execute_weekly_profiles()
        .await
        .expect("ETF 基本資料回補失敗");
}

/// 淨值、推斷分類、績效與快照。
#[tokio::test]
#[ignore]
async fn test_backfill_etf_daily() {
    dotenvy::dotenv().ok();
    etf_zone::execute_daily().await.expect("ETF 每日更新失敗");
}

/// 只重建快照（週末也會執行）。
#[tokio::test]
#[ignore]
async fn test_refresh_etf_snapshot() {
    use crate::domain::etf::EtfRepository;

    dotenvy::dotenv().ok();
    let rows = crate::infra::database::repository::etf::PgEtfRepository::new()
        .refresh_snapshot()
        .await
        .expect("ETF 快照重建失敗");
    println!("ETF 快照重建: rows={rows}");
}
