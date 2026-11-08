//! 異動計畫的套用的測試。

use rust_decimal_macros::dec;

use super::super::fixtures::announcement;
use super::*;

fn event(
    cash: Option<rust_decimal::Decimal>,
    ex_date: (i32, u32, u32),
    reason: UnresolvedReason,
) -> UnresolvedEvent {
    UnresolvedEvent {
        announcement: announcement("00904", ex_date, true, false, cash, None),
        reason,
    }
}

/// 2026-10-09 的 00904：10-16 除息、金額待公告、Yahoo 未列出 → 只是在等公告。
#[test]
fn test_is_awaiting_amount_far_from_ex_date() {
    let today = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
    let item = event(
        None,
        (2026, 10, 16),
        UnresolvedReason::NoMatchingYahooDividend,
    );

    assert!(is_awaiting_amount(&item, today));
}

/// 剩 3 天內還判不出來就要告警，免得錯過除息提醒。
#[test]
fn test_is_awaiting_amount_warns_near_ex_date() {
    let item = event(
        None,
        (2026, 10, 16),
        UnresolvedReason::NoMatchingYahooDividend,
    );

    let near = NaiveDate::from_ymd_opt(2026, 10, 13).unwrap();
    assert!(!is_awaiting_amount(&item, near));
    let day_before = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    assert!(is_awaiting_amount(&item, day_before));
}

/// 金額已公布卻對不上、或 Yahoo 抓取失敗，都不是等待狀態。
#[test]
fn test_is_awaiting_amount_requires_pending_amount_and_yahoo_miss() {
    let today = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();

    let announced = event(
        Some(dec!(0.5)),
        (2026, 10, 16),
        UnresolvedReason::NoMatchingYahooDividend,
    );
    assert!(!is_awaiting_amount(&announced, today));

    let failed = event(None, (2026, 10, 16), UnresolvedReason::YahooLookupFailed);
    assert!(!is_awaiting_amount(&failed, today));
}

/// 寫入計畫：更新日期、新增分期並重算年度合計；無法判定期別的事件只計數不寫入。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn apply_plan_updates_inserts_and_rebuilds_annual_total() {
    use std::collections::BTreeMap;

    use super::super::fixtures::existing_row;
    use crate::infra::database::repository::dividend::PgDividendRepository;

    const SYMBOL: &str = "79979AS";
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 apply_plan_updates_inserts_and_rebuilds_annual_total：無資料庫連接");
        return;
    }
    let cleanup = || async {
        sqlx::query("DELETE FROM dividend WHERE security_code = $1")
            .bind(SYMBOL)
            .execute(pool)
            .await
            .expect("清除測試股利");
    };
    cleanup().await;

    let repo = PgDividendRepository::new();
    let mut first = existing_row(0, SYMBOL, 1991, "Q1", "-", "-");
    first.cash_dividend = dec!(1);
    first.sum = dec!(1);
    repo.save(&first).await.expect("種入第一季");
    let mut update = repo
        .fetch_by_years(&[1991])
        .await
        .expect("讀取股利")
        .into_iter()
        .find(|row| row.security_code == SYMBOL)
        .expect("讀回第一季");
    update.ex_dividend_date_cash = "1991-04-10".to_string();
    let mut second = existing_row(0, SYMBOL, 1991, "Q2", "1991-07-10", "-");
    second.cash_dividend = dec!(2);
    second.sum = dec!(2);

    let plan = ScanPlan {
        updates: BTreeMap::from([(update.serial, update)]),
        inserts: BTreeMap::from([((SYMBOL.to_string(), 1991, 1990, "Q2".to_string()), second)]),
        unresolved: vec![
            event(
                None,
                (2099, 1, 1),
                UnresolvedReason::NoMatchingYahooDividend,
            ),
            event(None, (2099, 1, 1), UnresolvedReason::YahooLookupFailed),
        ],
    };
    let outcome = apply_plan(&repo, plan).await.expect("套用計畫");
    assert_eq!(
        (outcome.updated, outcome.inserted, outcome.unresolved),
        (1, 1, 2)
    );

    let rows: Vec<_> = repo
        .fetch_by_years(&[1991])
        .await
        .expect("重新讀取")
        .into_iter()
        .filter(|row| row.security_code == SYMBOL)
        .collect();
    let quarter = |q: &str| rows.iter().find(|row| row.quarter == q);
    assert_eq!(
        quarter("Q1").expect("第一季").ex_dividend_date_cash,
        "1991-04-10"
    );
    assert_eq!(quarter("Q2").expect("第二季").cash_dividend, dec!(2));
    let total = quarter("").or_else(|| quarter("A")).expect("年度合計列");
    assert_eq!(total.cash_dividend, dec!(3));

    cleanup().await;
}
