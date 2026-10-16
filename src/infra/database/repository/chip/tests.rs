//! 籌碼倉儲的去重與各表寫入、連續天數重算測試。

use rust_decimal_macros::dec;

use super::*;
use crate::domain::chip::BrokerNetRecord;
use crate::infra::database;

/// 測試用的假代號；寫入後一律清除。
const FAKE: &str = "79979CH";

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, 30).expect("日期應合法")
}

async fn cleanup() {
    for sql in [
        "DELETE FROM chip_daily WHERE stock_symbol = $1",
        "DELETE FROM holder_distribution WHERE stock_symbol = $1",
        "DELETE FROM insider_holding WHERE stock_symbol = $1",
        "DELETE FROM broker_flow WHERE stock_symbol = $1",
    ] {
        sqlx::query(sql)
            .bind(FAKE)
            .execute(database::get_connection())
            .await
            .expect("清除測試資料");
    }
}

/// 同一批重複的鍵只留最後一筆，順序不變。
#[test]
fn dedup_by_key_keeps_the_last_occurrence() {
    let items = [("a", 1), ("b", 2), ("a", 3)];
    let kept: Vec<_> = dedup_by_key(&items, |item| item.0)
        .into_iter()
        .copied()
        .collect();
    assert_eq!(kept, [("b", 2), ("a", 3)]);
}

/// 法人與融資融券寫進同一列、互不覆蓋；內容相同重送不算寫入；
/// 董監名單整批取代；分點的 JSON 能寫入並比對。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn saves_every_kind_of_chip_data() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 saves_every_kind_of_chip_data：無資料庫連接");
        return;
    }
    cleanup().await;
    let repo = PgChipRepository::new();

    let flow = InstitutionalNet {
        stock_symbol: FAKE.to_string(),
        foreign: 1_000,
        trust: -200,
        dealer: 30,
    };
    let margin = MarginRecord {
        stock_symbol: FAKE.to_string(),
        margin_previous: 10,
        margin_balance: 12,
        short_previous: 3,
        short_balance: 1,
    };
    assert_eq!(
        repo.save_institutional(day(), std::slice::from_ref(&flow))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        repo.save_margin(day(), std::slice::from_ref(&margin))
            .await
            .unwrap(),
        1
    );
    assert_eq!(repo.save_institutional(day(), &[flow]).await.unwrap(), 0);
    let row: (Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(
        r#"SELECT foreign_net, trust_net, margin_balance FROM chip_daily WHERE "date" = $1 AND stock_symbol = $2"#,
    )
    .bind(day())
    .bind(FAKE)
    .fetch_one(database::get_connection())
    .await
    .unwrap();
    assert_eq!(row, (Some(1_000), Some(-200), Some(12)));

    let holder = HolderDistributionRecord {
        date: day(),
        stock_symbol: FAKE.to_string(),
        major_holders: 50,
        major_percent: dec!(75.12),
        total_holders: 30_000,
    };
    assert_eq!(
        repo.save_holder_distributions(std::slice::from_ref(&holder))
            .await
            .unwrap(),
        1
    );
    assert_eq!(repo.save_holder_distributions(&[holder]).await.unwrap(), 0);

    let month = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
    let insider = |name: &str, pledged| InsiderHoldingRecord {
        month,
        stock_symbol: FAKE.to_string(),
        title: "董事".to_string(),
        name: name.to_string(),
        shares: 1_000,
        pledged,
        related_pledged: 0,
    };
    assert_eq!(
        repo.replace_insider_holdings(&[insider("甲", 0), insider("乙", 0)])
            .await
            .unwrap(),
        2
    );
    // 乙辭任、甲改為設質：刪 1、改 1。
    assert_eq!(
        repo.replace_insider_holdings(&[insider("甲", 500)])
            .await
            .unwrap(),
        2
    );
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM insider_holding WHERE stock_symbol = $1 ORDER BY name",
    )
    .bind(FAKE)
    .fetch_all(database::get_connection())
    .await
    .unwrap();
    assert_eq!(names, ["甲"]);

    let broker = BrokerFlowRecord {
        date: day(),
        stock_symbol: FAKE.to_string(),
        buy_total: 120,
        sell_total: 80,
        main_share: dec!(3.5),
        buyers: vec![BrokerNetRecord {
            name: "凱基-台北".to_string(),
            buy: 150,
            sell: 30,
            net: 120,
            share: dec!(6.98),
        }],
        sellers: Vec::new(),
    };
    assert_eq!(
        repo.save_broker_flows(std::slice::from_ref(&broker))
            .await
            .unwrap(),
        1
    );
    assert_eq!(repo.save_broker_flows(&[broker]).await.unwrap(), 0);

    cleanup().await;
}

/// 連續天數：同方向累加、反向重來、淨額 0 或沒有法人資料（只有融資融券）為 0；
/// 分段重算接續前一段最後一列，結果與整段重算相同；內容沒變時不更新任何列。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn refresh_streaks_counts_runs_and_resumes_from_previous_row() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 refresh_streaks_counts_runs_and_resumes_from_previous_row：無資料庫連接");
        return;
    }
    cleanup().await;
    let repo = PgChipRepository::new();
    let d = |n: u32| NaiveDate::from_ymd_opt(2001, 1, n).expect("日期應合法");
    let flow = |foreign, trust, dealer| InstitutionalNet {
        stock_symbol: FAKE.to_string(),
        foreign,
        trust,
        dealer,
    };
    for (n, f, t, dl) in [
        (2, 10, -5, 0),
        (3, 10, -5, 0),
        (4, -1, 0, 0),
        (8, 5, 5, 5),
        (9, 5, 5, 0),
    ] {
        repo.save_institutional(d(n), &[flow(f, t, dl)])
            .await
            .unwrap();
    }
    // 01-05 只有融資融券，法人欄位為 NULL。
    let margin = MarginRecord {
        stock_symbol: FAKE.to_string(),
        margin_previous: 1,
        margin_balance: 1,
        short_previous: 0,
        short_balance: 0,
    };
    repo.save_margin(d(5), &[margin]).await.unwrap();

    let streaks = || async {
        sqlx::query_as::<_, (Option<i16>, Option<i16>, Option<i16>, Option<i16>)>(
            r#"SELECT foreign_streak, trust_streak, dealer_streak, total_streak
               FROM chip_daily WHERE stock_symbol = $1 ORDER BY "date""#,
        )
        .bind(FAKE)
        .fetch_all(database::get_connection())
        .await
        .unwrap()
    };
    let expected = vec![
        (Some(1), Some(-1), Some(0), Some(1)),
        (Some(2), Some(-2), Some(0), Some(2)),
        (Some(-1), Some(0), Some(0), Some(-1)),
        (Some(0), Some(0), Some(0), Some(0)),
        (Some(1), Some(1), Some(1), Some(1)),
        (Some(2), Some(2), Some(0), Some(2)),
    ];

    assert_eq!(repo.refresh_streaks(d(1), d(31)).await.unwrap(), 6);
    assert_eq!(streaks().await, expected);
    assert_eq!(repo.refresh_streaks(d(1), d(31)).await.unwrap(), 0);

    sqlx::query(
        r#"UPDATE chip_daily SET foreign_streak = NULL, trust_streak = NULL, dealer_streak = NULL, total_streak = NULL
           WHERE stock_symbol = $1 AND "date" >= $2"#,
    )
    .bind(FAKE)
    .bind(d(3))
    .execute(database::get_connection())
    .await
    .unwrap();
    for (from, to) in [(d(3), d(3)), (d(4), d(5)), (d(6), d(9))] {
        repo.refresh_streaks(from, to).await.unwrap();
    }
    assert_eq!(streaks().await, expected);

    cleanup().await;
}
