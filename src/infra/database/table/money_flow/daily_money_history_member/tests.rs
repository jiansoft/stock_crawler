//! 會員每日市值表的測試。

use super::*;
use jiff::civil::Date;
use jiff::civil::date;
use jiff_sqlx::ToSqlx;

#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn pending_registration_has_no_settlement_rows() {
    let date = date(1987, 7, 23);
    let mut tx = Some(database::get_tx().await.expect("開啟隔離測試交易"));
    let conn = tx.as_mut().expect("交易存在");
    let household: i64 = sqlx::query_scalar(
        "INSERT INTO member_household (name, registration_pending)
         VALUES ('__settlement_pending__', true) RETURNING id",
    )
    .fetch_one(&mut **conn)
    .await
    .expect("建立待驗證的新家庭");
    sqlx::query(
        r#"INSERT INTO "Member" ("Account", household_id, is_active, activated_at, "CreateTime")
           VALUES ('__settlement_pending__', $1, false, NULL, '1987-07-01')"#,
    )
    .bind(household)
    .execute(&mut **conn)
    .await
    .expect("建立待驗證的會員");

    DailyMoneyHistoryMember::upsert(date, &mut tx)
        .await
        .expect("執行每日市值結算");

    let conn = tx.as_mut().expect("交易存在");
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM daily_money_history_member WHERE date=$1 AND household_id=$2",
    )
    .bind(date.to_sqlx())
    .bind(household)
    .fetch_one(&mut **conn)
    .await
    .expect("查詢暫存家庭是否產生合計或會員列");
    assert_eq!(count, 0);
}

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

    let date = date(1987, 7, 22);
    let previous = date.yesterday().expect("有前一日");
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
        let member: i64 = sqlx::query_scalar(
            r#"INSERT INTO "Member" ("Account", "Name", household_id, is_active, "CreateTime")
            VALUES ($1, $1, $2, $3, '1987-07-01') RETURNING "Id""#,
        )
        .bind(account)
        .bind(household)
        .bind(active)
        .fetch_one(&mut **conn)
        .await
        .expect("新增測試會員");
        members.push(member);
        sqlx::query("INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, date) VALUES ($1, 'IAM7SET', $2, -100, '1987-07-01')")
            .bind(member).bind(shares).execute(&mut **conn).await.expect("新增持股");
        if deleted {
            // 模擬早於刪除守門的殘留持股；已刪除會員現在無法直接新增持股。
            sqlx::query(r#"UPDATE "Member" SET deleted_at=now() WHERE "Id"=$1"#)
                .bind(member)
                .execute(&mut **conn)
                .await
                .expect("標記舊會員已刪除");
        }
    }
    sqlx::query(r#"INSERT INTO "DailyQuotes" (stock_symbol, "Date", "ClosingPrice") VALUES ('IAM7SET', $1, 20), ('IAM7SET', $2, 10)"#)
        .bind(date.to_sqlx()).bind(previous.to_sqlx()).execute(&mut **conn).await.expect("新增測試報價");
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
        .bind(date.to_sqlx()).bind(other).fetch_all(&mut **conn).await.expect("讀取家庭合計");
    assert_eq!(
        totals,
        [(1, dec!(200)), (other, dec!(600))],
        "停用資產仍計入自己的家庭，刪除者不混入"
    );
    let details: Vec<(i64, Decimal, Decimal, Decimal)> = sqlx::query_as("SELECT household_id, market_value, total_shares::numeric, ratio FROM daily_money_history_detail WHERE date=$1 AND security_code='IAM7SET' AND member_id=0 ORDER BY household_id")
        .bind(date.to_sqlx()).fetch_all(&mut **conn).await.expect("讀取家庭明細");
    assert_eq!(
        details,
        [
            (1, dec!(200), dec!(10), dec!(100)),
            (other, dec!(600), dec!(30), dec!(100))
        ]
    );
    let batches: Vec<(i64, Decimal)> = sqlx::query_as("SELECT household_id, sum(market_value) FROM daily_money_history_detail_more WHERE date=$1 AND security_code='IAM7SET' AND member_id=0 GROUP BY household_id ORDER BY household_id")
        .bind(date.to_sqlx()).fetch_all(&mut **conn).await.expect("讀取家庭交易批次");
    assert_eq!(batches, totals, "交易批次合計不得跨家庭重複連結");
    let comparisons = sqlx::query_as::<_, DailyMoneyHistoryMemberWithPreviousTradingDay>(
        PREVIOUS_TRADING_DAY_SQL,
    )
    .bind(date.to_sqlx())
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
            .bind(date.to_sqlx())
            .fetch_one(&mut **conn)
            .await
            .expect("讀取預設家庭舊表");
    assert_eq!(legacy_total, dec!(200));
    let deleted_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM daily_money_history_member WHERE date=$1 AND member_id=$2",
    )
    .bind(date.to_sqlx())
    .bind(members[2])
    .fetch_one(&mut **conn)
    .await
    .expect("確認刪除者無結算");
    assert_eq!(deleted_rows, 0);
    // 新結算明確保存結算家庭；搬家後可見家庭改變，原合計及歸屬仍須保留。
    let mismatch: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (
            SELECT household_id, settlement_household_id FROM daily_money_history_member WHERE date=$1
            UNION ALL SELECT household_id, settlement_household_id FROM daily_money_history_detail WHERE date=$1
            UNION ALL SELECT household_id, settlement_household_id FROM daily_money_history_detail_more WHERE date=$1
         ) rows WHERE household_id<>settlement_household_id"
    ).bind(date.to_sqlx()).fetch_one(&mut **conn).await.expect("確認新結算歸屬");
    assert_eq!(mismatch, 0, "新結算的三張市值表必須保存原家庭");
    sqlx::query("SELECT public.member_money_history_move($1, $2)")
        .bind(members[0])
        .bind(other)
        .execute(&mut **conn)
        .await
        .expect("搬移本人歷史");
    let moved: (i64, i64) = sqlx::query_as("SELECT household_id, settlement_household_id FROM daily_money_history_member WHERE date=$1 AND member_id=$2")
        .bind(date.to_sqlx()).bind(members[0]).fetch_one(&mut **conn).await.expect("讀取搬移後歸屬");
    assert_eq!(moved, (other, 1));
    let original: Decimal = sqlx::query_scalar("SELECT market_value FROM daily_money_history_member WHERE date=$1 AND household_id=1 AND member_id=0")
        .bind(date.to_sqlx()).fetch_one(&mut **conn).await.expect("原家庭歷史合計");
    assert_eq!(original, dec!(200), "搬家不重算歷史合計");
    sqlx::query("SELECT public.member_money_history_delete($1)")
        .bind(members[0])
        .execute(&mut **conn)
        .await
        .expect("刪除後重建原結算家庭");
    let removed_total: Decimal = sqlx::query_scalar("SELECT market_value FROM daily_money_history_member WHERE date=$1 AND household_id=1 AND member_id=0")
        .bind(date.to_sqlx()).fetch_one(&mut **conn).await.expect("刪除後原家庭合計");
    let removed_legacy: Decimal =
        sqlx::query_scalar("SELECT sum FROM daily_money_history WHERE date=$1")
            .bind(date.to_sqlx())
            .fetch_one(&mut **conn)
            .await
            .expect("刪除後凍結寬表合計");
    assert_eq!(
        (removed_total, removed_legacy),
        (Decimal::ZERO, Decimal::ZERO)
    );
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
    let current_date = Date::strptime("%Y-%m-%d", "2023-08-30").unwrap();
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
    let date = date(1987, 7, 20);
    let mut tx = Some(database::get_tx().await.expect("開啟交易"));
    let mut ids = Vec::new();
    {
        let conn = tx.as_mut().expect("交易已開啟");
        // (帳號, 啟用, 建立時間)；__dmh_deleted__ 在種入持股後才標記刪除
        for (account, active, created) in [
            ("__dmh_active__", true, "1987-07-01 12:00+08"),
            ("__dmh_inactive__", false, "1987-07-01 12:00+08"),
            ("__dmh_deleted__", true, "1987-07-01 12:00+08"),
            ("__dmh_future__", true, "1987-07-21 12:00+08"),
        ] {
            let id: i64 = sqlx::query_scalar(
                r#"INSERT INTO "Member" ("Account", "Name", household_id, is_active, "CreateTime")
                   VALUES ($1, $1, 1, $2, $3::timestamptz)
                   RETURNING "Id""#,
            )
            .bind(account)
            .bind(active)
            .bind(created)
            .fetch_one(&mut **conn)
            .await
            .expect("建立測試會員");
            ids.push(id);
        }
        // 已刪除的會員即使還有持股列也不計入。隱私守門不允許替已刪除會員寫入持股，
        // 所以先以未刪除狀態種入持股，再標記刪除（與實際「持股在先、刪除在後」的順序相同）。
        sqlx::query(
            "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, date)
             VALUES ($1, '79979SA', 1, '1987-07-01')",
        )
        .bind(ids[2])
        .execute(&mut **conn)
        .await
        .expect("種入待刪除會員的持股");
        sqlx::query(r#"UPDATE "Member" SET deleted_at = now() WHERE "Id" = $1"#)
            .bind(ids[2])
            .execute(&mut **conn)
            .await
            .expect("標記會員已刪除");
    }

    DailyMoneyHistoryMember::upsert(date, &mut tx)
        .await
        .expect("會員市值結算");
    let conn = tx.as_mut().expect("交易已開啟");
    let rows: Vec<(i64, Decimal)> = sqlx::query_as(
        "SELECT member_id, market_value FROM daily_money_history_member
         WHERE date = $1 AND member_id = ANY($2) ORDER BY member_id",
    )
    .bind(date.to_sqlx())
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
