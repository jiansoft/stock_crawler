//! 策略換股紀錄（[`StrategyRebalanceRepository`]）的 PostgreSQL 實作的測試。

use super::*;

/// 寫入後依日期由舊到新讀回；同策略同一天再寫不覆蓋；其他策略的紀錄不混入。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn save_keeps_first_record_and_lists_by_date() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 save_keeps_first_record_and_lists_by_date：無資料庫連接");
        return;
    }
    let name = "79979 換股紀錄測試";
    let cleanup = || async {
        sqlx::query("DELETE FROM strategy_rebalance WHERE preset_name = ANY($1)")
            .bind([name, "79979 其他策略"])
            .execute(database::get_connection())
            .await
            .expect("清除測試資料");
    };
    cleanup().await;
    let day = |d| NaiveDate::from_ymd_opt(1995, 3, d).expect("有效日期");
    let record = |d, symbol: &str| StrategyRebalanceRecord {
        preset_name: name.to_owned(),
        date: day(d),
        query: "from=1995-01-02&limit=2".to_owned(),
        hold_days: 20,
        etf_weight: 50,
        cash: 10.0,
        market_filtered: false,
        holdings: vec![
            RecordedHolding {
                symbol: symbol.to_owned(),
                name: Some("甲".to_owned()),
                weight: 60.0,
            },
            RecordedHolding {
                symbol: "79979RB".to_owned(),
                name: None,
                weight: 30.0,
            },
        ],
    };
    let repository = PgStrategyRebalanceRepository::new();
    assert!(repository.save(&record(20, "79979RA")).await.expect("寫入"));
    assert!(repository.save(&record(1, "79979RC")).await.expect("寫入"));
    assert!(
        !repository
            .save(&record(20, "79979RZ"))
            .await
            .expect("重複寫入")
    );
    assert!(
        repository
            .save(&StrategyRebalanceRecord {
                preset_name: "79979 其他策略".to_owned(),
                ..record(5, "79979RA")
            })
            .await
            .expect("其他策略")
    );

    let records = repository.list(name).await.expect("讀取");
    cleanup().await;
    assert_eq!(records, [record(1, "79979RC"), record(20, "79979RA")]);
}
