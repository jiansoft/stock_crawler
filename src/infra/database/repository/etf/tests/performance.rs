//! ETF performance 主題的倉儲驗證。

use super::*;

/// 真實 repository 寫入後立刻經共用快取讀取；測試本身不呼叫 invalidate。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn committed_profile_and_nav_writes_invalidate_cached_detail() {
    use crate::domain::etf::query::EtfQueryRepository;
    use crate::infra::{
        cache::etf_analysis::ETF_ANALYSIS, database::repository::etf_query::PgEtfQueryRepository,
    };
    cleanup().await;
    let repo = PgEtfRepository::new();
    let query = PgEtfQueryRepository::new();
    sqlx::query(
        "INSERT INTO etf_snapshot(stock_symbol,name,market_id) VALUES ($1,'快取測試 ETF',2)",
    )
    .bind(FAKE)
    .execute(database::get_connection())
    .await
    .unwrap();
    let before = EtfProfileRecord {
        stock_symbol: FAKE.into(),
        full_name: "before".into(),
        source: "yahoo".into(),
        ..Default::default()
    };
    repo.save_profiles(std::slice::from_ref(&before))
        .await
        .unwrap();
    let cached = ETF_ANALYSIS
        .detail(FAKE, 20, query.etf_detail(FAKE, 20))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cached.profile.unwrap().full_name, "before");
    assert!(cached.navs.is_empty());
    repo.save_profiles(&[EtfProfileRecord {
        full_name: "after".into(),
        ..before
    }])
    .await
    .unwrap();
    let fresh = ETF_ANALYSIS
        .detail(FAKE, 20, query.etf_detail(FAKE, 20))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fresh.profile.unwrap().full_name, "after");
    repo.insert_nav_history(
        FAKE,
        &[EtfNavHistoryPoint {
            date: day(30),
            nav: dec!(10),
            market_price: None,
        }],
    )
    .await
    .unwrap();
    let fresh = ETF_ANALYSIS
        .detail(FAKE, 20, query.etf_detail(FAKE, 20))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fresh.navs.len(), 1);
    assert_eq!(fresh.navs[0].nav, Some(10.0));
    // 即使不重建 snapshot，每日淨值 writer 仍必須使明細失效。
    repo.save_navs(&[nav(day(30), 100, None)]).await.unwrap();
    let fresh = ETF_ANALYSIS
        .detail(FAKE, 20, query.etf_detail(FAKE, 20))
        .await
        .unwrap()
        .unwrap();
    assert!(fresh.navs[0].units_change.is_some());
    cleanup().await;
}
