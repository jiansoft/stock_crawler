//! 會員每日市值表的測試。

use super::*;

/// 兩個家庭共持同股票時，三張表、損益占比與前日合計都不可交叉；停用仍計入，刪除不計入。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn household_settlement_isolated_postgres() {
    use crate::infra::database::table::money_flow::{
        daily_money_history::DailyMoneyHistory,
        daily_money_history_detail::DailyMoneyHistoryDetail,
        daily_money_history_detail_more::DailyMoneyHistoryDetailMore,
    };
    use rust_decimal_macros::dec;

    let date = NaiveDate::from_ymd_opt(1987, 7, 22).expect("固定日期合法");
    let previous = date.pred_opt().expect("有前一日");
    let mut tx = Some(database::get_tx().await.expect("開啟隔離測試交易"));
    let conn = tx.as_mut().expect("交易存在");
    let other: i64 = sqlx::query_scalar(
        "INSERT INTO member_household (name) VALUES ('__settlement_other__') RETURNING id",
    )
    .fetch_one(&mut **conn)
    .await
    .expect("新增第二家庭");
    let mut members = Vec::new();
    for (account, household, active, deleted, shares) in [
        ("__settlement_a__", 1, true, false, 10_i64),
        ("__settlement_b__", other, false, false, 30),
        ("__settlement_deleted__", other, true, true, 100),
    ] {
        let member: i64 = sqlx::query_scalar(r#"INSERT INTO "Member" ("Account", "Name", household_id, is_active, deleted_at, "CreateTime")
            VALUES ($1, $1, $2, $3, CASE WHEN $4 THEN now() END, '1987-07-01') RETURNING "Id""#)
            .bind(account).bind(household).bind(active).bind(deleted)
            .fetch_one(&mut **conn).await.expect("新增測試會員");
        members.push(member);
        sqlx::query("INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, date) VALUES ($1, 'IAM7SET', $2, -100, '1987-07-01')")
            .bind(member).bind(shares).execute(&mut **conn).await.expect("新增持股");
    }
    sqlx::query(r#"INSERT INTO "DailyQuotes" (stock_symbol, "Date", "ClosingPrice") VALUES ('IAM7SET', $1, 20), ('IAM7SET', $2, 10)"#)
        .bind(date).bind(previous).execute(&mut **conn).await.expect("新增測試報價");
    for day in [previous, date] {
        DailyMoneyHistoryMember::upsert(day, &mut tx)
            .await
            .expect("逐家庭結算總覽");
    }
    DailyMoneyHistoryDetail::upsert(date, &mut tx)
        .await
        .expect("逐家庭結算明細");
    DailyMoneyHistoryDetailMore::upsert(date, &mut tx)
        .await
        .expect("逐家庭結算批次");
    DailyMoneyHistory::upsert(date, &mut tx)
        .await
        .expect("舊表只結算預設家庭");
    let conn = tx.as_mut().expect("交易存在");
    let totals: Vec<(i64, Decimal)> = sqlx::query_as("SELECT household_id, market_value FROM daily_money_history_member WHERE date=$1 AND member_id=0 AND household_id IN (1, $2) ORDER BY household_id")
        .bind(date).bind(other).fetch_all(&mut **conn).await.expect("讀取家庭合計");
    assert_eq!(
        totals,
        [(1, dec!(200)), (other, dec!(600))],
        "停用資產仍計入自己的家庭，刪除者不混入"
    );
    let details: Vec<(i64, Decimal, Decimal, Decimal)> = sqlx::query_as("SELECT household_id, market_value, total_shares::numeric, ratio FROM daily_money_history_detail WHERE date=$1 AND security_code='IAM7SET' AND member_id=0 ORDER BY household_id")
        .bind(date).fetch_all(&mut **conn).await.expect("讀取家庭明細");
    assert_eq!(
        details,
        [
            (1, dec!(200), dec!(10), dec!(100)),
            (other, dec!(600), dec!(30), dec!(100))
        ]
    );
    let batches: Vec<(i64, Decimal)> = sqlx::query_as("SELECT household_id, sum(market_value) FROM daily_money_history_detail_more WHERE date=$1 AND security_code='IAM7SET' AND member_id=0 GROUP BY household_id ORDER BY household_id")
        .bind(date).fetch_all(&mut **conn).await.expect("讀取家庭交易批次");
    assert_eq!(batches, totals, "交易批次合計不得跨家庭重複連結");
    let comparisons = sqlx::query_as::<_, DailyMoneyHistoryMemberWithPreviousTradingDay>(
        PREVIOUS_TRADING_DAY_SQL,
    )
    .bind(date)
    .fetch_all(&mut **conn)
    .await
    .expect("家庭前日比較");
    for (household, current, previous_value) in
        [(1, dec!(200), dec!(100)), (other, dec!(600), dec!(300))]
    {
        let row = comparisons
            .iter()
            .find(|row| row.household_id == household && row.member_id == 0)
            .expect("家庭合計存在");
        assert_eq!(
            (row.market_value, row.previous_market_value),
            (current, previous_value),
            "合計不能只用 member_id=0 配對"
        );
    }
    let legacy_total: Decimal =
        sqlx::query_scalar("SELECT sum FROM daily_money_history WHERE date=$1")
            .bind(date)
            .fetch_one(&mut **conn)
            .await
            .expect("讀取預設家庭舊表");
    assert_eq!(legacy_total, dec!(200));
    let deleted_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM daily_money_history_member WHERE date=$1 AND member_id=$2",
    )
    .bind(date)
    .bind(members[2])
    .fetch_one(&mut **conn)
    .await
    .expect("確認刪除者無結算");
    assert_eq!(deleted_rows, 0);
    let implicit_defaults: i64 = sqlx::query_scalar("SELECT count(*) FROM information_schema.columns WHERE table_schema='public' AND ((table_name IN ('daily_money_history_member','daily_money_history_detail','daily_money_history_detail_more') AND column_name='household_id') OR (table_name='trace' AND column_name='member_id')) AND column_default IS NOT NULL")
        .fetch_one(&mut **conn).await.expect("確認隱含擁有者預設值移除");
    assert_eq!(implicit_defaults, 0);
    tx.take()
        .expect("交易存在")
        .rollback()
        .await
        .expect("回滾所有測試資料");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_upsert() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_upsert：無資料庫連接");
        return;
    }
    tracing::debug!("開始 DailyMoneyHistoryMember::upsert");
    let current_date = NaiveDate::parse_from_str("2023-08-30", "%Y-%m-%d").unwrap();
    let mut tx = database::get_tx().await.ok();

    match DailyMoneyHistoryMember::upsert(current_date, &mut tx).await {
        Ok(r) => {
            tracing::debug!("DailyMoneyHistoryMember::upsert:{:#?}", r);
            if let Some(tx) = tx {
                tx.commit().await.expect("commit is failed");
            }
        }
        Err(why) => {
            tracing::debug!(
                "Failed to DailyMoneyHistoryMember::upsert because {:?}",
                why
            );
            if let Some(tx) = tx {
                tx.rollback().await.expect("rollback is failed");
            }
        }
    }

    tracing::debug!("結束 DailyMoneyHistoryMember::upsert");
}

/// 結算對象：未刪除的會員（含已停用）即使沒有持股也有每日市值列；已刪除的不計入；
/// 建立日晚於結算日的會員不會出現在較早的日期。全部在交易內完成並回滾。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_upsert_covers_all_undeleted_members() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_upsert_covers_all_undeleted_members：無資料庫連接");
        return;
    }
    let date = NaiveDate::from_ymd_opt(1987, 7, 20).expect("固定日期合法");
    let mut tx = Some(database::get_tx().await.expect("開啟交易"));
    let mut ids = Vec::new();
    {
        let conn = tx.as_mut().expect("交易已開啟");
        // (帳號, 啟用, 已刪除, 建立時間)
        for (account, active, deleted, created) in [
            ("__dmh_active__", true, false, "1987-07-01 12:00+08"),
            ("__dmh_inactive__", false, false, "1987-07-01 12:00+08"),
            ("__dmh_deleted__", true, true, "1987-07-01 12:00+08"),
            ("__dmh_future__", true, false, "1987-07-21 12:00+08"),
        ] {
            let id: i64 = sqlx::query_scalar(
                r#"INSERT INTO "Member" ("Account", "Name", household_id, is_active, deleted_at, "CreateTime")
                   VALUES ($1, $1, 1, $2, CASE WHEN $3 THEN now() END, $4::timestamptz)
                   RETURNING "Id""#,
            )
            .bind(account)
            .bind(active)
            .bind(deleted)
            .bind(created)
            .fetch_one(&mut **conn)
            .await
            .expect("建立測試會員");
            ids.push(id);
        }
        // 已刪除的會員即使還有持股列也不計入
        sqlx::query(
            "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, date)
             VALUES ($1, '79979SA', 1, '1987-07-01')",
        )
        .bind(ids[2])
        .execute(&mut **conn)
        .await
        .expect("種入已刪除會員的持股");
    }

    DailyMoneyHistoryMember::upsert(date, &mut tx)
        .await
        .expect("會員市值結算");
    let conn = tx.as_mut().expect("交易已開啟");
    let rows: Vec<(i64, Decimal)> = sqlx::query_as(
        "SELECT member_id, market_value FROM daily_money_history_member
         WHERE date = $1 AND member_id = ANY($2) ORDER BY member_id",
    )
    .bind(date)
    .bind(&ids)
    .fetch_all(&mut **conn)
    .await
    .expect("讀取會員市值");
    tx.take()
        .expect("交易已開啟")
        .rollback()
        .await
        .expect("回滾測試資料");

    assert_eq!(rows, vec![(ids[0], Decimal::ZERO), (ids[1], Decimal::ZERO)]);
}
