//! CAGR 資料來源的異常跳動偵測測試：除息、上市初期與公司行動能解釋的跳動不標記。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_anomaly_detection_ignores_jumps_explained_by_dividends() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_anomaly_detection_ignores_jumps_explained_by_dividends：無資料庫連接");
        return;
    }
    seed().await;
    insert_listing_history(FAKE_A).await;
    let repo = PgCagrSourceRepository::new();

    let events = repo
        .fetch_anomaly_events(day(2), day(8))
        .await
        .expect("fetch_anomaly_events");
    let mine: Vec<NaiveDate> = events
        .iter()
        .filter(|(symbol, _)| symbol == FAKE_A)
        .map(|(_, date)| *date)
        .collect();

    // 01-03 跳 60% 但當天有登錄除息日 → 可解釋，不標記；
    // 01-05 跳 87.5% 且無對應除權息 → 標記；
    // 01-08 僅跳 3.3%，未達 30% 門檻。
    assert_eq!(mine, vec![day(5)]);

    // 分桶併發掃描與不分桶的單一語句結果必須相同。
    let mut unbucketed = fetch_anomaly_bucket(day(2), day(8), (1, 0))
        .await
        .expect("不分桶掃描應成功");
    let mut bucketed = events.clone();
    unbucketed.sort();
    bucketed.sort();
    assert_eq!(bucketed, unbucketed);

    // 事件帶日期回傳，呼叫端才能依期間各自判定：縮小區間後該事件應消失。
    let narrowed = repo
        .fetch_anomaly_events(day(5), day(8))
        .await
        .expect("fetch_anomaly_events narrowed");
    assert!(
        !narrowed
            .iter()
            .any(|(symbol, date)| symbol == FAKE_A && *date == day(5)),
        "區間起點之後才計算跳動，起點當天不應再被標記"
    );

    cleanup().await;
}

/// 上市初期（前 5 筆報價）的跳動不算異常；公司行動生效日沒有報價時，
/// 只要落在前一筆報價之後、這一筆（含）之前就算已解釋。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_anomaly_detection_skips_listing_days_and_matches_actions_between_quotes() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!(
            "跳過 test_anomaly_detection_skips_listing_days_and_matches_actions_between_quotes：無資料庫連接"
        );
        return;
    }
    cleanup().await;
    insert_stock(FAKE_C, false).await;
    // 01-03 是上市第 2 筆、漲一倍：上市初期，不算異常。
    for (d, close) in [
        (2, dec!(10)),
        (3, dec!(20)),
        (4, dec!(20)),
        (5, dec!(20)),
        (8, dec!(20)),
    ] {
        insert_quote(FAKE_C, day(d), close).await;
    }
    // 01-09 生效的減資當天沒有成交，只有沿用停牌前價格的零量補值列（1538 2024-12-30 的情況），
    // 01-10 才有成交、價格跳 1.5 倍：補值列不算前一筆報價，仍視為已解釋。
    insert_corporate_action(FAKE_C, day(9), dec!(0.4)).await;
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "TradingVolume")
           VALUES ($1, $2, 20, 0)"#,
    )
    .bind(day(9))
    .bind(FAKE_C)
    .execute(database::get_connection())
    .await
    .expect("插入零量補值列");
    insert_quote(FAKE_C, day(10), dec!(50)).await;
    // 01-12 再跳一倍，沒有任何事件：異常。
    insert_quote(FAKE_C, day(12), dec!(100)).await;

    let events = PgCagrSourceRepository::new()
        .fetch_anomaly_events(day(1), day(31))
        .await
        .expect("fetch_anomaly_events");
    cleanup().await;

    let mine: Vec<NaiveDate> = events
        .iter()
        .filter(|(symbol, _)| symbol == FAKE_C)
        .map(|(_, date)| *date)
        .collect();
    assert_eq!(mine, vec![day(12)]);
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_corporate_actions_are_fetched_and_suppress_anomaly_flags() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!(
            "跳過 test_corporate_actions_are_fetched_and_suppress_anomaly_flags：無資料庫連接"
        );
        return;
    }
    seed().await;
    insert_listing_history(FAKE_A).await;
    let repo = PgCagrSourceRepository::new();

    // 尚未登錄時，01-05 那次 87.5% 的跳動會被標記為異常。
    let before = repo
        .fetch_anomaly_events(day(2), day(8))
        .await
        .expect("fetch_anomaly_events");
    assert!(
        before
            .iter()
            .any(|(symbol, date)| symbol == FAKE_A && *date == day(5))
    );

    insert_corporate_action(FAKE_A, day(5), dec!(0.5)).await;

    let actions = repo
        .fetch_corporate_actions_since(day(1))
        .await
        .expect("fetch_corporate_actions_since");
    let mine: Vec<&CorporateAction> = actions
        .iter()
        .filter(|action| action.stock_symbol == FAKE_A)
        .collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].effective_date, day(5));
    assert_eq!(mine[0].share_ratio, dec!(0.5));

    // 登錄之後同一天不得再被視為異常——事件已建模，旗標應該熄滅。
    let after = repo
        .fetch_anomaly_events(day(2), day(8))
        .await
        .expect("fetch_anomaly_events again");
    assert!(
        !after
            .iter()
            .any(|(symbol, date)| symbol == FAKE_A && *date == day(5)),
        "已登錄的分割不該再被標記為疑似異常"
    );

    // since 之後才生效者才回傳。
    let later = repo
        .fetch_corporate_actions_since(day(5))
        .await
        .expect("fetch_corporate_actions_since later");
    assert!(!later.iter().any(|action| action.stock_symbol == FAKE_A));

    cleanup().await;
}
