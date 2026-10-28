//! 目標配置歷史版本的測試：比對規則（單元）與儲存、清除、取得（隔離資料庫）。

use rust_decimal_macros::dec;

use super::*;
use crate::domain::portfolio::target::PortfolioTargetRepository;
use crate::infra::database;

fn targets(items: &[(&str, f64)]) -> Vec<(String, f64)> {
    items
        .iter()
        .map(|(key, percent)| ((*key).to_string(), *percent))
        .collect()
}

/// 比對到兩位小數、不看順序；多一項、少一項或比例不同都算有變。
#[test]
fn unchanged_compares_two_decimal_sets_regardless_of_order() {
    let current = vec![
        ("A".to_string(), dec!(60.00)),
        ("B".to_string(), dec!(30.50)),
    ];
    assert!(unchanged(&current, &targets(&[("B", 30.5), ("A", 60.0)])));
    assert!(!unchanged(&current, &targets(&[("A", 60.0), ("B", 30.0)])));
    assert!(!unchanged(&current, &targets(&[("A", 60.0)])));
    assert!(!unchanged(
        &current,
        &targets(&[("A", 60.0), ("B", 30.5), ("C", 1.0)])
    ));
    assert!(unchanged(&[], &[]));
}

/// 第一次設定不記版本；之後每次內容有變就記下覆蓋前的整組（含全現金），內容相同不記；
/// 只保留最近 20 版；別的會員取不到。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn save_records_previous_targets_and_keeps_latest_twenty() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 save_records_previous_targets_and_keeps_latest_twenty：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let member = "79977";
    let cleanup = || async {
        for sql in [
            "DELETE FROM portfolio_target WHERE member_id='79977'",
            "DELETE FROM portfolio_target_dimension WHERE member_id='79977'",
            "DELETE FROM portfolio_target_revision WHERE member_id='79977'",
        ] {
            sqlx::query(sql).execute(pool).await.unwrap();
        }
    };
    cleanup().await;
    let repo = PgPortfolioTargetRepository::with_pool(pool.clone());
    let stock = TargetDimension::Stock;

    repo.save_targets(member, stock, &targets(&[("79977A", 60.0)]))
        .await
        .unwrap();
    assert!(repo.list_revisions(member, stock).await.unwrap().is_empty());

    repo.save_targets(
        member,
        stock,
        &targets(&[("79977A", 40.0), ("79977B", 30.25)]),
    )
    .await
    .unwrap();
    // 內容相同（順序不同）不記。
    repo.save_targets(
        member,
        stock,
        &targets(&[("79977B", 30.25), ("79977A", 40.0)]),
    )
    .await
    .unwrap();
    // 全現金也是一組設定：記下覆蓋前的兩檔。
    repo.save_targets(member, stock, &[]).await.unwrap();
    repo.save_targets(member, stock, &targets(&[("79977C", 10.0)]))
        .await
        .unwrap();

    let revisions = repo.list_revisions(member, stock).await.unwrap();
    let contents: Vec<Vec<(String, f64)>> = revisions.iter().map(|r| r.targets.clone()).collect();
    assert_eq!(
        contents,
        [
            vec![],
            targets(&[("79977A", 40.0), ("79977B", 30.25)]),
            targets(&[("79977A", 60.0)]),
        ]
    );
    assert!(revisions.iter().all(|r| r.dimension == stock));

    let oldest = revisions.last().unwrap().clone();
    assert_eq!(
        repo.get_revision(member, oldest.id).await.unwrap(),
        Some(oldest.clone())
    );
    assert_eq!(repo.get_revision("79976", oldest.id).await.unwrap(), None);
    // 別的維度不受影響。
    assert!(
        repo.list_revisions(member, TargetDimension::Industry)
            .await
            .unwrap()
            .is_empty()
    );

    for percent in 1..=25 {
        repo.save_targets(member, stock, &targets(&[("79977D", f64::from(percent))]))
            .await
            .unwrap();
    }
    let revisions = repo.list_revisions(member, stock).await.unwrap();
    assert_eq!(revisions.len(), MAX_TARGET_REVISIONS as usize);
    // 最新一版是 79977D 24%（25% 是目前的設定）；最舊的三版已清掉。
    assert_eq!(revisions[0].targets, targets(&[("79977D", 24.0)]));
    assert!(
        repo.get_revision(member, oldest.id)
            .await
            .unwrap()
            .is_none()
    );

    cleanup().await;
}
