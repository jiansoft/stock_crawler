//! 使用隔離資料庫驗證整份設定的交易語意。

use super::*;
use crate::domain::portfolio::view::HoldingView;

/// 持股載入：加總未售出股數、帶出收盤價／產業／ETF 旗標，並為目標中未持有的代號補零股列。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn fetch_positions_joins_prices_and_pads_missing_targets() {
    let pool = database::get_connection();
    let repo = PgPortfolioTargetRepository::with_pool(pool.clone());
    let cleanup = [
        "DELETE FROM stock_ownership_details WHERE member_id=79978",
        "DELETE FROM last_daily_quotes WHERE stock_symbol IN ('79978A','79978B','79978C')",
        "DELETE FROM stocks WHERE stock_symbol IN ('79978A','79978B','79978C')",
    ];
    for sql in cleanup {
        sqlx::query(sql).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO stocks (\"SecurityCode\",\"Name\",stock_symbol,stock_exchange_market_id,stock_industry_id) VALUES ('79978A','甲股','79978A',2,24),('79978B','乙股','79978B',2,24),('79978C','丙股','79978C',2,24)")
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO last_daily_quotes(stock_symbol,date,closing_price) VALUES ('79978A','2026-10-23',100),('79978C','2026-10-23',50)")
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO stock_ownership_details(member_id,security_code,share_quantity,is_sold) VALUES (79978,'79978A',1000,false),(79978,'79978A',500,false),(79978,'79978B',2000,false),(79978,'79978B',9000,true)")
        .execute(pool).await.unwrap();

    let positions = repo
        .fetch_positions(
            &HoldingView::member(1, 79978),
            &["79978A".to_owned(), "79978C".to_owned()],
        )
        .await
        .unwrap();
    let by = |symbol: &str| positions.iter().find(|p| p.symbol == symbol);
    let a = by("79978A").expect("持有 A");
    assert_eq!((a.shares, a.current_price), (1500, 100.0));
    assert_eq!(a.name, "甲股");
    assert!(!a.is_etf);
    // 已售出的不計；B 沒有收盤價時價格為 0，由呼叫端的試算拒絕。
    let b = by("79978B").expect("持有 B");
    assert_eq!((b.shares, b.current_price), (2000, 0.0));
    // 目標中未持有的 C 補上零股列並帶價格。
    let c = by("79978C").expect("補上 C");
    assert_eq!((c.shares, c.current_price), (0, 50.0));
    // 已持有的目標不重複補列。
    assert_eq!(positions.iter().filter(|p| p.symbol == "79978A").count(), 1);

    for sql in cleanup {
        sqlx::query(sql).execute(pool).await.unwrap();
    }
}

/// ETF 成分：只取前十大、帶出名稱／權重／資料日期，查無主檔的成分產業為空字串；沒有 ETF 時不查詢。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn fetch_etf_components_returns_top_ten_with_dates() {
    let pool = database::get_connection();
    let repo = PgPortfolioTargetRepository::with_pool(pool.clone());
    let cleanup = [
        "DELETE FROM etf_holding WHERE stock_symbol='79978EF'",
        "DELETE FROM stocks WHERE stock_symbol='79978A'",
    ];
    for sql in cleanup {
        sqlx::query(sql).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO stocks (\"SecurityCode\",\"Name\",stock_symbol,stock_exchange_market_id,stock_industry_id) VALUES ('79978A','甲股','79978A',2,24)")
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO etf_holding(stock_symbol,rank,holding_symbol,holding_name,weight,as_of) VALUES ('79978EF',1,'79978A','甲股',40,'2026-04-30'),('79978EF',2,'79978Z','乙成分',12.5,'2026-04-30'),('79978EF',11,'79978Y','十一名',1,'2026-04-30')")
        .execute(pool).await.unwrap();

    assert!(repo.fetch_etf_components(&[]).await.unwrap().is_empty());
    let components = repo
        .fetch_etf_components(&["79978EF".to_owned()])
        .await
        .unwrap();
    let as_of = chrono::NaiveDate::from_ymd_opt(2026, 4, 30).unwrap();
    let summary: Vec<_> = components
        .iter()
        .map(|dated| {
            (
                dated.component.stock_symbol.as_str(),
                dated.component.name.as_str(),
                dated.component.weight_percent,
                dated.component.etf_symbol.as_str(),
                dated.as_of,
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            ("79978A", "甲股", 40.0, "79978EF", as_of),
            ("79978Z", "乙成分", 12.5, "79978EF", as_of),
        ]
    );
    assert!(components[1].component.industry.is_empty());

    for sql in cleanup {
        sqlx::query(sql).execute(pool).await.unwrap();
    }
}

/// 並行覆蓋不得合併成 120%，驗證失敗也不得清掉已存設定。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn targets_replace_atomically_and_preserve_cash_configuration() {
    let repo = PgPortfolioTargetRepository::with_pool(database::get_connection().clone());
    let owner = TargetOwner::new(1, "79979-target-regression".to_owned());
    let member = &owner;
    for sql in [
        "DELETE FROM portfolio_target WHERE member_id=$1",
        "DELETE FROM portfolio_target_dimension WHERE member_id=$1",
    ] {
        sqlx::query(sql)
            .bind(&member.member_id)
            .execute(repo.pool())
            .await
            .unwrap();
    }
    assert!(
        !repo
            .has_configuration(member, TargetDimension::Stock)
            .await
            .unwrap()
    );
    let a = vec![("A".to_owned(), 60.0)];
    let b = vec![("B".to_owned(), 60.0)];
    let actor = TargetActor::default();
    let (a, b) = tokio::join!(
        repo.save_targets(member, TargetDimension::Stock, &a, &actor),
        repo.save_targets(member, TargetDimension::Stock, &b, &actor)
    );
    a.unwrap();
    b.unwrap();
    let stored = repo
        .list_targets(member, Some(TargetDimension::Stock))
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].target_percent, 60.0);
    assert!(
        repo.save_targets(
            member,
            TargetDimension::Stock,
            &[
                ("A".into(), 33.335),
                ("B".into(), 33.335),
                ("C".into(), 33.33)
            ],
            &TargetActor::default()
        )
        .await
        .is_err()
    );
    assert_eq!(
        repo.list_targets(member, Some(TargetDimension::Stock))
            .await
            .unwrap()
            .len(),
        1
    );
    repo.save_targets(
        member,
        TargetDimension::Stock,
        &[("A".into(), 0.0)],
        &TargetActor::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        repo.list_targets(member, Some(TargetDimension::Stock))
            .await
            .unwrap()[0]
            .target_percent,
        0.0
    );
    repo.save_targets(member, TargetDimension::Stock, &[], &TargetActor::default())
        .await
        .unwrap();
    assert!(
        repo.has_configuration(member, TargetDimension::Stock)
            .await
            .unwrap()
    );
    assert!(
        repo.list_targets(member, Some(TargetDimension::Stock))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repo.configured_dimensions(member).await.unwrap(),
        [TargetDimension::Stock]
    );
    for sql in [
        "DELETE FROM portfolio_target WHERE member_id=$1",
        "DELETE FROM portfolio_target_dimension WHERE member_id=$1",
    ] {
        sqlx::query(sql)
            .bind(&member.member_id)
            .execute(repo.pool())
            .await
            .unwrap();
    }
}

/// 讀寫依家庭分開：A 家庭的全家目標在 B 家庭讀不到、版本不能跨家庭取用；歷史版本記下寫入來源。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要隔離 PostgreSQL")]
async fn targets_are_scoped_by_household_and_revisions_record_actor() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 targets_are_scoped_by_household_and_revisions_record_actor：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let repo = PgPortfolioTargetRepository::with_pool(pool.clone());
    // 假家庭編號，不碰開發庫裡家庭 #1 的全家目標
    let (first, second) = (979_901_i64, 979_902_i64);
    cleanup_households(pool, &[first, second]).await;
    let a = TargetOwner::new(first, "0".to_owned());
    let b = TargetOwner::new(second, "0".to_owned());
    let stock = TargetDimension::Stock;
    let web = TargetActor::parse(Some("web"), Some("1")).unwrap();
    let mcp = TargetActor::parse(Some("mcp"), Some("0193a7b2-token")).unwrap();

    repo.save_targets(&a, stock, &[("79979A".into(), 50.0)], &web)
        .await
        .unwrap();
    assert!(repo.list_targets(&b, Some(stock)).await.unwrap().is_empty());
    assert!(!repo.has_configuration(&b, stock).await.unwrap());
    assert!(repo.configured_dimensions(&b).await.unwrap().is_empty());

    // 第二次寫入：覆蓋前的那一組記成一版，帶上這次的來源
    repo.save_targets(&a, stock, &[("79979C".into(), 20.0)], &mcp)
        .await
        .unwrap();
    let revisions = repo.list_revisions(&a, stock).await.unwrap();
    assert_eq!(revisions.len(), 1);
    assert_eq!(revisions[0].targets, [("79979A".to_owned(), 50.0)]);
    assert_eq!(revisions[0].actor, mcp);
    assert!(repo.list_revisions(&b, stock).await.unwrap().is_empty());
    assert_eq!(repo.get_revision(&b, revisions[0].id).await.unwrap(), None);

    cleanup_households(pool, &[first, second]).await;
}

/// 兩個家庭各有一份 `member_id = '0'` 的全家目標時互不覆蓋。
///
/// 過渡唯一索引 `idx_portfolio_target_dimension_legacy_key`（member_id, dimension）讓 L1 之後退回舊版
/// 仍能儲存，但也擋住第二個家庭的同一會員與維度；Phase 7 移除過渡索引時拿掉 `ignore` 啟用這個測試。
#[tokio::test]
#[ignore = "Phase 7 移除過渡唯一索引 idx_portfolio_target_dimension_legacy_key 後啟用"]
async fn family_targets_of_two_households_do_not_overwrite() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 family_targets_of_two_households_do_not_overwrite：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let repo = PgPortfolioTargetRepository::with_pool(pool.clone());
    let (first, second) = (979_901_i64, 979_902_i64);
    cleanup_households(pool, &[first, second]).await;
    let a = TargetOwner::new(first, "0".to_owned());
    let b = TargetOwner::new(second, "0".to_owned());
    let stock = TargetDimension::Stock;
    let actor = TargetActor::default();
    repo.save_targets(&a, stock, &[("79979A".into(), 50.0)], &actor)
        .await
        .unwrap();
    repo.save_targets(&b, stock, &[("79979B".into(), 30.0)], &actor)
        .await
        .unwrap();
    let keys = |records: Vec<PortfolioTargetRecord>| -> Vec<String> {
        records.into_iter().map(|record| record.key).collect()
    };
    assert_eq!(
        keys(repo.list_targets(&a, Some(stock)).await.unwrap()),
        ["79979A"]
    );
    assert_eq!(
        keys(repo.list_targets(&b, Some(stock)).await.unwrap()),
        ["79979B"]
    );
    cleanup_households(pool, &[first, second]).await;
}

/// 清除假家庭的配置目標、維度與版本。
async fn cleanup_households(pool: &sqlx::PgPool, households: &[i64]) {
    for sql in [
        "DELETE FROM portfolio_target WHERE household_id = ANY($1)",
        "DELETE FROM portfolio_target_dimension WHERE household_id = ANY($1)",
        "DELETE FROM portfolio_target_revision WHERE household_id = ANY($1)",
    ] {
        sqlx::query(sql)
            .bind(households)
            .execute(pool)
            .await
            .unwrap();
    }
}
