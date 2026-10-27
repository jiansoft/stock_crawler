//! ETF holding 主題的倉儲驗證。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn heuristic_profiles_never_overwrite_source_profiles() {
    dotenvy::dotenv().ok();
    cleanup().await;
    let repo = PgEtfRepository::new();
    let heuristic = EtfProfileRecord {
        stock_symbol: FAKE.to_string(),
        strategy: "thematic".to_string(),
        region: "台灣".to_string(),
        source: "heuristic".to_string(),
        ..Default::default()
    };
    assert_eq!(
        repo.insert_missing_profiles(std::slice::from_ref(&heuristic))
            .await
            .expect("補推斷分類"),
        1
    );
    // 推斷規則改變時，推斷列跟著更新。
    let updated = EtfProfileRecord {
        strategy: "high_dividend".to_string(),
        ..heuristic.clone()
    };
    assert_eq!(
        repo.insert_missing_profiles(std::slice::from_ref(&updated))
            .await
            .expect("更新推斷分類"),
        1
    );

    let yahoo = EtfProfileRecord {
        stock_symbol: FAKE.to_string(),
        full_name: "測試基金".to_string(),
        strategy: "market_cap".to_string(),
        themes: vec!["半導體".to_string()],
        nav_return: Some(serde_json::json!({"1y": 12.5})),
        total_assets: Some(dec!(1000)),
        source: "yahoo".to_string(),
        ..Default::default()
    };
    assert_eq!(
        repo.save_profiles(std::slice::from_ref(&yahoo))
            .await
            .expect("寫入來源資料"),
        1
    );
    assert_eq!(
        repo.insert_missing_profiles(&[heuristic])
            .await
            .expect("推斷不覆蓋來源"),
        0
    );

    let (strategy, themes, source): (String, Vec<String>, String) =
        sqlx::query_as("SELECT strategy, themes, source FROM etf_profile WHERE stock_symbol = $1")
            .bind(FAKE)
            .fetch_one(database::get_connection())
            .await
            .expect("讀回基本資料");
    assert_eq!(strategy, "market_cap");
    assert_eq!(themes, vec!["半導體".to_string()]);
    assert_eq!(source, "yahoo");
    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn holding_history_keeps_every_period() {
    dotenvy::dotenv().ok();
    cleanup().await;
    let repo = PgEtfRepository::new();
    let holding = |rank, symbol: &str, weight| EtfHoldingRecord {
        rank,
        holding_symbol: symbol.to_string(),
        holding_name: format!("名稱{symbol}"),
        weight,
    };
    let portfolio = |as_of, holdings| EtfPortfolio {
        stock_symbol: FAKE.to_string(),
        holdings_as_of: Some(as_of),
        holdings,
        ..Default::default()
    };
    let symbols = [FAKE.to_string()];
    let since = NaiveDate::from_ymd_opt(2000, 1, 1).expect("日期應合法");

    repo.replace_portfolio(&portfolio(day(1), vec![holding(1, "2330", dec!(50))]))
        .await
        .expect("第一期");
    assert!(
        repo.holding_periods(&symbols, since)
            .await
            .expect("查詢")
            .is_empty(),
        "只有一期時沒有可比"
    );
    // 同一期重抓只更正內容；再收到新一期。
    repo.replace_portfolio(&portfolio(day(1), vec![holding(1, "2330", dec!(51))]))
        .await
        .expect("同期重抓");
    repo.replace_portfolio(&portfolio(
        day(30),
        vec![holding(1, "2330", dec!(55)), holding(2, "2308", dec!(3))],
    ))
    .await
    .expect("新一期");

    let periods = repo.holding_periods(&symbols, since).await.expect("查詢");
    assert_eq!(periods.len(), 1);
    let p = &periods[0];
    assert_eq!((p.previous.as_of, p.current.as_of), (day(1), day(30)));
    assert_eq!(p.previous.holdings[0].weight, dec!(51.0000));
    assert_eq!(p.current.holdings.len(), 2);
    // 最新一期在 since 之前收到的不列。
    let future = NaiveDate::from_ymd_opt(2099, 1, 1).expect("日期應合法");
    assert!(
        repo.holding_periods(&symbols, future)
            .await
            .expect("查詢")
            .is_empty()
    );
    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn replace_portfolio_keeps_sections_the_source_did_not_return() {
    dotenvy::dotenv().ok();
    cleanup().await;
    let repo = PgEtfRepository::new();
    let allocation = |label: &str, weight: Decimal| EtfAllocationRecord {
        label: label.to_string(),
        weight,
    };
    let full = EtfPortfolio {
        stock_symbol: FAKE.to_string(),
        holdings_as_of: Some(day(1)),
        holdings: vec![EtfHoldingRecord {
            rank: 1,
            holding_symbol: "2330".to_string(),
            holding_name: "台積電".to_string(),
            weight: dec!(50),
        }],
        industries_as_of: Some(day(17)),
        // 同名的列只留第一筆。
        industries: vec![
            allocation("半導體業", dec!(60)),
            allocation("半導體業", dec!(1)),
            allocation("金融保險", dec!(10)),
        ],
        assets_as_of: Some(day(17)),
        assets: vec![allocation("股票", dec!(99))],
    };
    assert_eq!(repo.replace_portfolio(&full).await.expect("寫入持股"), 4);

    // 新一期只有資產分佈：前十大與行業比重保留舊資料。
    let partial = EtfPortfolio {
        stock_symbol: FAKE.to_string(),
        assets_as_of: Some(day(24)),
        assets: vec![allocation("股票", dec!(98)), allocation("現金", dec!(2))],
        ..Default::default()
    };
    assert_eq!(repo.replace_portfolio(&partial).await.expect("部分更新"), 2);

    let count = |sql: &'static str| async move {
        let (n,): (i64,) = sqlx::query_as(sql)
            .bind(FAKE)
            .fetch_one(database::get_connection())
            .await
            .expect("計數");
        n
    };
    assert_eq!(
        count("SELECT COUNT(*) FROM etf_holding WHERE stock_symbol = $1").await,
        1
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM etf_allocation WHERE stock_symbol = $1 AND kind = 'industry'")
            .await,
        2
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM etf_allocation WHERE stock_symbol = $1 AND kind = 'asset'")
            .await,
        2
    );
    cleanup().await;
}
