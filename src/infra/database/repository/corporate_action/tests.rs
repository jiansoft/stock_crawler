//! 公司行動（分割／減資）的 PostgreSQL 倉儲實作的測試。

use super::*;
use rust_decimal_macros::dec;

const FAKE_SYMBOL: &str = "79979CA";

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
}

async fn cleanup() {
    let _ = sqlx::query("DELETE FROM corporate_action WHERE stock_symbol = $1")
        .bind(FAKE_SYMBOL)
        .execute(database::get_connection())
        .await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_save_is_idempotent_and_persists_action_type() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_save_is_idempotent_and_persists_action_type：無資料庫連接");
        return;
    }

    let repo = PgCorporateActionRepository::new();
    cleanup().await;

    let mut action = CorporateAction {
        stock_symbol: FAKE_SYMBOL.to_string(),
        effective_date: date(1990, 1, 5),
        action_type: CorporateActionType::Split,
        share_ratio: dec!(4),
        note: "1:4 分割".to_string(),
    };
    assert_eq!(repo.save(&action).await.expect("save"), 1);

    let saved = repo
        .fetch_by_symbol(FAKE_SYMBOL)
        .await
        .expect("fetch_by_symbol");
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].share_ratio, dec!(4));
    assert_eq!(saved[0].note, "1:4 分割");

    // 同一主鍵重送是修正而非新增。
    action.action_type = CorporateActionType::CapitalReduction;
    action.share_ratio = dec!(0.7);
    action.note = "更正為減資三成".to_string();
    repo.save(&action).await.expect("save again");

    let saved = repo
        .fetch_by_symbol(FAKE_SYMBOL)
        .await
        .expect("fetch_by_symbol again");
    assert_eq!(saved.len(), 1, "重複登錄不應新增資料列");
    assert_eq!(saved[0].share_ratio, dec!(0.7));

    assert_eq!(saved[0].action_type, CorporateActionType::CapitalReduction);

    // 型別以呼叫端指定者為準。
    let action_type: String =
        sqlx::query_scalar("SELECT action_type FROM corporate_action WHERE stock_symbol = $1")
            .bind(FAKE_SYMBOL)
            .fetch_one(database::get_connection())
            .await
            .expect("query action_type");
    assert_eq!(action_type, "capital_reduction");

    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_by_symbol_is_sorted_and_scoped() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_fetch_by_symbol_is_sorted_and_scoped：無資料庫連接");
        return;
    }

    let repo = PgCorporateActionRepository::new();
    cleanup().await;

    for (day, ratio) in [(9_u32, dec!(2)), (3_u32, dec!(4))] {
        repo.save(&CorporateAction {
            stock_symbol: FAKE_SYMBOL.to_string(),
            effective_date: date(1990, 1, day),
            action_type: CorporateActionType::Split,
            share_ratio: ratio,
            note: String::new(),
        })
        .await
        .expect("save");
    }

    let saved = repo
        .fetch_by_symbol(FAKE_SYMBOL)
        .await
        .expect("fetch_by_symbol");
    assert_eq!(saved.len(), 2);
    assert_eq!(saved[0].effective_date, date(1990, 1, 3));
    assert_eq!(saved[1].effective_date, date(1990, 1, 9));

    // 其他代號查不到這批資料。
    assert!(
        repo.fetch_by_symbol("79979CB")
            .await
            .expect("fetch other symbol")
            .is_empty()
    );

    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_by_effective_date_returns_only_that_day() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_fetch_by_effective_date_returns_only_that_day：無資料庫連接");
        return;
    }

    let repo = PgCorporateActionRepository::new();
    cleanup().await;

    for (day, ratio) in [(9_u32, dec!(0.5)), (10_u32, dec!(0.7))] {
        repo.save(&CorporateAction {
            stock_symbol: FAKE_SYMBOL.to_string(),
            effective_date: date(1990, 3, day),
            action_type: CorporateActionType::CapitalReduction,
            share_ratio: ratio,
            note: "減資彌補虧損".to_string(),
        })
        .await
        .expect("save");
    }

    let found = repo
        .fetch_by_effective_date(date(1990, 3, 9))
        .await
        .expect("fetch_by_effective_date");
    let ours: Vec<_> = found
        .iter()
        .filter(|action| action.stock_symbol == FAKE_SYMBOL)
        .collect();
    assert_eq!(ours.len(), 1);
    assert_eq!(ours[0].share_ratio, dec!(0.5));
    assert_eq!(ours[0].action_type, CorporateActionType::CapitalReduction);

    cleanup().await;
}

/// 迴歸測試：減資退還股款的比例可能大於 1（參考價已扣掉退還的現金），
/// 舊實作以 `share_ratio < 1` 反推型別會把它誤存成 `split`。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_save_keeps_capital_reduction_when_ratio_exceeds_one() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_save_keeps_capital_reduction_when_ratio_exceeds_one：無資料庫連接");
        return;
    }

    let repo = PgCorporateActionRepository::new();
    cleanup().await;

    // 取自 8201 無敵 2016-07-18：前收 8.69、恢復買賣參考價 8.12。
    repo.save(&CorporateAction {
        stock_symbol: FAKE_SYMBOL.to_string(),
        effective_date: date(1990, 2, 1),
        action_type: CorporateActionType::CapitalReduction,
        share_ratio: dec!(1.0702),
        note: "減資退還股款".to_string(),
    })
    .await
    .expect("save");

    let saved = repo
        .fetch_by_symbol(FAKE_SYMBOL)
        .await
        .expect("fetch_by_symbol");
    assert_eq!(saved.len(), 1);
    assert_eq!(
        saved[0].action_type,
        CorporateActionType::CapitalReduction,
        "比例大於 1 仍須維持減資型別"
    );

    cleanup().await;
}
