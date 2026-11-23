//! money_flow的測試；連外的測試標 #[ignore]。

use super::*;
use crate::infra::database;
use jiff::civil::date;
use jiff_sqlx::ToSqlx;

#[tokio::test]
#[ignore]
async fn test_money_flow_repository_flow() {
    // 載入環境變數
    dotenvy::dotenv().ok();

    if database::ping().await.is_err() {
        println!("跳過 test_money_flow_repository_flow：無資料庫連接");
        return;
    }

    // 建立資金流向與市值倉儲實例
    let repo = PgMoneyFlowRepository::new();
    // 選擇有完整資料之特定測試交易日
    let test_date = date(2026, 4, 20);

    // 1. 執行 recalculate_and_save_money_flow，驗證多表交易式批次更新是否可順利執行
    let result = repo.recalculate_and_save_money_flow(test_date).await;
    assert!(
        result.is_ok(),
        "recalculate_and_save_money_flow failed: {:?}",
        result.err()
    );

    // 2. 驗證讀取會員對照資料是否正常
    let compare_data = repo
        .fetch_member_money_history_with_previous_day(test_date)
        .await;
    assert!(
        compare_data.is_ok(),
        "fetch_member_money_history_with_previous_day failed: {:?}",
        compare_data.err()
    );
    let compare_data = compare_data.unwrap();
    // 預期至少有合計列 (member_id = 0)
    assert!(!compare_data.is_empty());
    assert!(compare_data.iter().any(|r| r.member_id == 0));
}

/// 固定歷史日期：重算流程整段 transaction 可執行，會員市值對照取最近兩個有資料的日期。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn money_flow_repository_pairs_member_values_with_previous_day() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 money_flow_repository_pairs_member_values_with_previous_day：無資料庫連接");
        return;
    }
    const MEMBER: i64 = 979_790;
    let previous = date(1990, 3, 1);
    let current = date(1990, 3, 2);
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM daily_money_history_detail_more WHERE date = ANY($1)",
            "DELETE FROM daily_money_history_detail WHERE date = ANY($1)",
            "DELETE FROM daily_money_history_member WHERE date = ANY($1)",
            "DELETE FROM daily_money_history WHERE date = ANY($1)",
            "DELETE FROM daily_stock_price_stats WHERE date = ANY($1)",
        ] {
            let _ = sqlx::query(sql)
                .bind(
                    ([previous, current])
                        .iter()
                        .map(|value| value.to_sqlx())
                        .collect::<Vec<_>>(),
                )
                .execute(pool)
                .await;
        }
    };
    cleanup().await;

    let repo = PgMoneyFlowRepository::new();
    repo.recalculate_and_save_money_flow(current)
        .await
        .expect("重算流程應成功");

    // 以種入的會員市值取代重算結果，驗證對照：前一日 100、當日 120。
    cleanup().await;
    sqlx::query(
        "INSERT INTO daily_money_history_member (household_id, settlement_household_id, date, member_id, market_value)
         VALUES (1, 1, $1, $3, 100), (1, 1, $2, $3, 120)",
    )
    .bind(previous.to_sqlx())
    .bind(current.to_sqlx())
    .bind(MEMBER)
    .execute(pool)
    .await
    .expect("種入會員市值");
    let rows = repo
        .fetch_member_money_history_with_previous_day(current)
        .await
        .expect("讀取會員對照");
    let row = rows
        .iter()
        .find(|row| row.member_id == MEMBER)
        .expect("應有測試會員");
    assert_eq!((row.date, row.previous_date), (current, Some(previous)));
    assert_eq!(
        (row.market_value, row.previous_market_value),
        (
            rust_decimal::Decimal::from(120),
            rust_decimal::Decimal::from(100)
        )
    );

    cleanup().await;
}

/// 結算輸出快照：三張市值表加上 household_id、改主鍵之後，輸出必須與改動前逐字相同。
///
/// 全部在同一個交易內種資料、結算、讀取，最後 ROLLBACK，不留任何資料。
/// 預期值由改動前的程式產生；`household_id` 欄在改動前來自欄位預設值 1。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn settlement_output_matches_snapshot() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 settlement_output_matches_snapshot：無資料庫連接");
        return;
    }
    const MEMBER_A: i64 = 979_801;
    const MEMBER_B: i64 = 979_802;
    let current = date(1987, 7, 16);
    let mut tx = Some(database::get_tx().await.expect("開啟交易"));

    {
        let conn = tx.as_mut().expect("交易已開啟");
        // 新結算必須從會員取得家庭，快照夾具明確建立兩位預設家庭會員。
        for member in [MEMBER_A, MEMBER_B] {
            sqlx::query(r#"INSERT INTO "Member" ("Id", "Account", "Name", household_id, "CreateTime")
                VALUES ($1, $2, $2, 1, '1987-01-01') ON CONFLICT ("Id") DO UPDATE SET household_id=1, deleted_at=NULL"#)
                .bind(member).bind(format!("__snapshot_{member}__"))
                .execute(&mut **conn).await.expect("建立快照會員");
        }
        for (date, symbol, price) in [
            ("1987-07-15", "79979SA", "10"),
            ("1987-07-16", "79979SA", "12"),
            ("1987-07-15", "79979SB", "20"),
            ("1987-07-16", "79979SB", "19"),
        ] {
            sqlx::query(
                r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice")
                   VALUES ($1::date, $2, $3::numeric)
                   ON CONFLICT (stock_symbol, "Date") DO UPDATE SET "ClosingPrice" = EXCLUDED."ClosingPrice""#,
            )
            .bind(date)
            .bind(symbol)
            .bind(price)
            .execute(&mut **conn)
            .await
            .expect("種入收盤價");
        }
        // 建立時間取台北中午，transaction_date 在任何連線時區都是同一天
        for (member, symbol, shares, cost, average, created) in [
            (
                MEMBER_A,
                "79979SA",
                1000_i64,
                "-10000",
                "10",
                "1987-04-01 12:00+08",
            ),
            (
                MEMBER_A,
                "79979SB",
                500,
                "-9500",
                "19",
                "1987-04-02 12:00+08",
            ),
            (
                MEMBER_B,
                "79979SA",
                2000,
                "-22000",
                "11",
                "1987-04-03 12:00+08",
            ),
        ] {
            sqlx::query(
                "INSERT INTO stock_ownership_details
                     (member_id, security_code, share_quantity, holding_cost, share_price_average, created_time, date)
                 VALUES ($1, $2, $3, $4::numeric, $5::numeric, $6::timestamptz, ($6::timestamptz AT TIME ZONE 'Asia/Taipei')::date)",
            )
            .bind(member)
            .bind(symbol)
            .bind(shares)
            .bind(cost)
            .bind(average)
            .bind(created)
            .execute(&mut **conn)
            .await
            .expect("種入庫存");
        }
    }

    TableDailyMoneyHistoryMember::upsert(current, &mut tx)
        .await
        .expect("會員市值結算");
    TableDailyMoneyHistoryDetail::delete(current, &mut tx)
        .await
        .expect("清除當日明細");
    TableDailyMoneyHistoryDetail::upsert(current, &mut tx)
        .await
        .expect("明細結算");
    TableDailyMoneyHistoryDetailMore::delete(current, &mut tx)
        .await
        .expect("清除當日批次明細");
    TableDailyMoneyHistoryDetailMore::upsert(current, &mut tx)
        .await
        .expect("批次明細結算");

    let conn = tx.as_mut().expect("交易已開啟");
    let members: Vec<String> = sqlx::query_scalar(
        "SELECT concat_ws('|', household_id, member_id, market_value)
         FROM daily_money_history_member
         WHERE date = $1 AND household_id = 1 AND member_id IN (0, $2, $3)
         ORDER BY household_id, member_id",
    )
    .bind(current.to_sqlx())
    .bind(MEMBER_A)
    .bind(MEMBER_B)
    .fetch_all(&mut **conn)
    .await
    .expect("讀取會員市值");
    let details: Vec<String> = sqlx::query_scalar(
        "SELECT concat_ws('|', household_id, member_id, security_code, closing_price, total_shares, cost,
                          average_unit_price_per_share, market_value, ratio, transfer_tax, profit_and_loss,
                          profit_and_loss_percentage, previous_day_market_value,
                          previous_day_profit_and_loss, previous_day_profit_and_loss_percentage)
         FROM daily_money_history_detail
         WHERE date = $1 AND security_code LIKE '79979S%'
         ORDER BY household_id, member_id, security_code",
    )
    .bind(current.to_sqlx())
    .fetch_all(&mut **conn)
    .await
    .expect("讀取明細");
    let lots: Vec<String> = sqlx::query_scalar(
        "SELECT concat_ws('|', household_id, member_id, transaction_date, security_code, closing_price,
                          number_of_shares_held, unit_price_per_share, cost, market_value,
                          profit_and_loss, profit_and_loss_percentage)
         FROM daily_money_history_detail_more
         WHERE date = $1 AND security_code LIKE '79979S%'
         ORDER BY household_id, member_id, security_code, transaction_date",
    )
    .bind(current.to_sqlx())
    .fetch_all(&mut **conn)
    .await
    .expect("讀取批次明細");
    tx.take()
        .expect("交易已開啟")
        .rollback()
        .await
        .expect("回滾測試資料");

    assert_eq!(members, SNAPSHOT_MEMBERS);
    assert_eq!(details, SNAPSHOT_DETAILS);
    assert_eq!(lots, SNAPSHOT_LOTS);
}

/// 欄位：household_id|member_id|market_value
const SNAPSHOT_MEMBERS: [&str; 3] = [
    "1|0|45500.0000",
    "1|979801|21500.0000",
    "1|979802|24000.0000",
];

/// 欄位：household_id|member_id|security_code|closing_price|total_shares|cost|
/// average_unit_price_per_share|market_value|ratio|transfer_tax|profit_and_loss|
/// profit_and_loss_percentage|previous_day_market_value|previous_day_profit_and_loss|
/// previous_day_profit_and_loss_percentage
const SNAPSHOT_DETAILS: [&str; 5] = [
    "1|0|79979SA|12.0000|3000|-32000.0000|10.6667|36000.0000|79.1209|108.0000|4000.0000|12.5000|30000.0000|6000.0000|20.0000",
    "1|0|79979SB|19.0000|500|-9500.0000|19.0000|9500.0000|20.8791|28.5000|0.0000|0.0000|10000.0000|-500.0000|-5.0000",
    "1|979801|79979SA|12.0000|1000|-10000.0000|10.0000|12000.0000|55.8140|36.0000|2000.0000|20.0000|10000.0000|2000.0000|20.0000",
    "1|979801|79979SB|19.0000|500|-9500.0000|19.0000|9500.0000|44.1860|28.5000|0.0000|0.0000|10000.0000|-500.0000|-5.0000",
    "1|979802|79979SA|12.0000|2000|-22000.0000|11.0000|24000.0000|100.0000|72.0000|2000.0000|9.0909|20000.0000|4000.0000|20.0000",
];

/// 欄位：household_id|member_id|transaction_date|security_code|closing_price|
/// number_of_shares_held|unit_price_per_share|cost|market_value|profit_and_loss|
/// profit_and_loss_percentage
const SNAPSHOT_LOTS: [&str; 6] = [
    "1|0|1987-04-01|79979SA|12.0000|1000|10.0000|-10000.0000|12000.0000|2000.0000|20.0000",
    "1|0|1987-04-03|79979SA|12.0000|2000|11.0000|-22000.0000|24000.0000|2000.0000|9.0909",
    "1|0|1987-04-02|79979SB|19.0000|500|19.0000|-9500.0000|9500.0000|0.0000|0.0000",
    "1|979801|1987-04-01|79979SA|12.0000|1000|10.0000|-10000.0000|12000.0000|2000.0000|20.0000",
    "1|979801|1987-04-02|79979SB|19.0000|500|19.0000|-9500.0000|9500.0000|0.0000|0.0000",
    "1|979802|1987-04-03|79979SA|12.0000|2000|11.0000|-22000.0000|24000.0000|2000.0000|9.0909",
];
