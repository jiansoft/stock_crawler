//! 漲跌幅／成交量排行 endpoint 的資料庫語意整合測試與假資料。

use super::*;

/// 漲跌幅／成交量排行的真實資料庫語意整合測試（movers 計畫 §4）。
///
/// 這個測試跑在**非交易時段的收盤來源路徑**上：即時快取為空時
/// endpoint 必須回 `source = "closing"`，並且各欄位的單位與缺值語意
/// 完全依 §4.4 決定。覆蓋參數 422、三種排序、市場過濾、名次連續性與
/// 「收盤來源不得出現張數／快照時間」等契約。只讀不寫；無資料庫連線時
/// 安全跳過。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn movers_endpoint_db_semantics() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 movers_endpoint_db_semantics：無資料庫連接");
        return;
    }
    // 這個測試驗證的是「非交易時段」語意，若殘留即時快照會讓來源變成
    // realtime，因此先確認快取為空，否則直接跳過而不是清空別人的快取。
    if !crate::infra::cache::SHARE.stock_snapshots_are_empty() {
        println!("跳過 movers_endpoint_db_semantics：即時快照非空");
        return;
    }
    // CI 的測試庫沒有日報價，以固定歷史日期與假代號種入資料，讓收盤路徑真的跑到
    // 排序與過濾。資料庫已有更新的交易日時排行以那一天為準，種入的資料只用於下方
    // 「data_as_of 是種入日期」時的精確斷言。
    let seed_date = chrono::NaiveDate::from_ymd_opt(2026, 4, 30).expect("固定日期合法");
    movers_seed::cleanup(seed_date).await;
    movers_seed::seed(seed_date).await;
    let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
        let generated = "movers-integration-test-key".to_owned();
        unsafe { std::env::set_var("DATA_API_KEY", &generated) };
        generated
    });
    let get = |path: String| {
        let key = key.clone();
        async move {
            let response = router()
                .oneshot(
                    Request::get(&path)
                        .header("Authorization", format!("Bearer {key}"))
                        .body(Body::empty())
                        .expect("request should build"),
                )
                .await
                .expect("router should serve request");
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body should be readable");
            let json: serde_json::Value =
                serde_json::from_slice(&bytes).expect("body should be JSON");
            (status, json)
        }
    };

    // 語意一：參數不合法 → 422（在任何 SQL 之前擋下）。
    for path in [
        "/api/v1/market/movers?rank_by=top_trade_value",
        "/api/v1/market/movers?rank_by=TOP_GAINERS",
        "/api/v1/market/movers?market=emerging",
        "/api/v1/market/movers?limit=0",
        "/api/v1/market/movers?limit=51",
    ] {
        let (status, _) = get(path.to_owned()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{path} 應回 422");
    }

    // 語意二：非交易時段固定走收盤來源，且缺值語意依 §4.4。
    let (status, json) = get("/api/v1/market/movers?limit=20".to_owned()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["source"], "closing");
    assert_eq!(json["is_realtime"], false);
    assert_eq!(json["rank_by"], "top_gainers");
    assert_eq!(json["market"], "all");
    assert_eq!(
        json["snapshot_updated_at"],
        serde_json::Value::Null,
        "收盤來源沒有快照時間，不可偽造"
    );
    let data_as_of = json["data_as_of"].as_str().expect("data_as_of 應為字串");
    assert_eq!(data_as_of.len(), 10, "data_as_of 應為 YYYY-MM-DD");
    let movers = json["movers"].as_array().expect("movers array");
    assert!(movers.len() <= 20);
    for (index, mover) in movers.iter().enumerate() {
        assert_eq!(mover["rank"], index as u64 + 1, "名次必須從一連續遞增");
        assert!(
            matches!(mover["market_id"].as_i64(), Some(2 | 4)),
            "all 只含上市與上櫃"
        );
        assert!(
            mover["volume_shares"].as_f64().unwrap_or_default() > 0.0,
            "零成交量必須被排除"
        );
        // 收盤來源沒有「張」與昨收，也沒有採集站點。
        assert_eq!(mover["volume_lots"], serde_json::Value::Null);
        assert_eq!(mover["last_close"], serde_json::Value::Null);
        assert_eq!(mover["source_site"], serde_json::Value::Null);
    }

    // 語意三：三種排序鍵的方向與同值穩定排序。
    for (rank_by, metric, descending) in [
        ("top_gainers", "change_percent", true),
        ("top_losers", "change_percent", false),
        ("top_volume", "volume_shares", true),
    ] {
        let (status, json) = get(format!("/api/v1/market/movers?rank_by={rank_by}&limit=50")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["rank_by"], rank_by);
        let movers = json["movers"].as_array().expect("movers array");
        for pair in movers.windows(2) {
            let left = pair[0][metric].as_f64().expect("指標應為數值");
            let right = pair[1][metric].as_f64().expect("指標應為數值");
            if descending {
                assert!(left >= right, "{rank_by} 必須由高到低");
            } else {
                assert!(left <= right, "{rank_by} 必須由低到高");
            }
            if left == right {
                assert!(
                    pair[0]["stock_symbol"].as_str() <= pair[1]["stock_symbol"].as_str(),
                    "{rank_by} 同值時必須依股票代號升冪"
                );
            }
        }
    }

    // 語意四：市場過濾不得混入其他市場。
    for (market, expected_id) in [("twse", 2), ("tpex", 4)] {
        let (status, json) = get(format!("/api/v1/market/movers?market={market}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["market"], market);
        assert!(
            json["movers"]
                .as_array()
                .expect("movers array")
                .iter()
                .all(|mover| mover["market_id"] == expected_id)
        );
    }

    // 語意五：最新交易日就是種入日期時（CI 的空測試庫），名次必須完全符合種入資料：
    // 暫停上市的 79983 與零成交量的 79984 都不得出現。
    if json["data_as_of"] == "2026-04-30" {
        let symbols = |json: &serde_json::Value| -> Vec<String> {
            json["movers"]
                .as_array()
                .expect("movers array")
                .iter()
                .filter_map(|mover| mover["stock_symbol"].as_str().map(str::to_owned))
                .collect()
        };
        for (query, expected) in [
            ("rank_by=top_gainers", vec!["79982", "79981"]),
            ("rank_by=top_losers", vec!["79981", "79982"]),
            ("rank_by=top_volume", vec!["79981", "79982"]),
            ("market=twse", vec!["79981"]),
            ("market=tpex", vec!["79982"]),
        ] {
            let (status, json) = get(format!("/api/v1/market/movers?{query}")).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(symbols(&json), expected, "{query} 的名次不符");
        }
        let (_, json) = get("/api/v1/market/movers?limit=1".to_owned()).await;
        let top = &json["movers"][0];
        assert_eq!(top["stock_symbol"], "79982");
        assert_eq!(top["change_percent"], 9.0);
        assert_eq!(top["volume_shares"], 500.0);
        assert_eq!(top["trade_value"], 50_000.0);
        assert_eq!(top["transaction"], 10.0);
    }

    movers_seed::cleanup(seed_date).await;
}

/// movers 整合測試的假資料：固定歷史日期、假代號，測試前後都會清除。
mod movers_seed {
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    /// `(代號, 市場, 暫停上市, 漲跌幅 %, 成交股數)`；昨收固定 100 元。
    const ROWS: [(&str, i32, bool, i64, i64); 4] = [
        ("79981", 2, false, 5, 1_000),
        ("79982", 4, false, 9, 500),
        ("79983", 2, true, 20, 300),
        ("79984", 2, false, 8, 0),
    ];

    /// 寫入股票主檔與指定日期的日報價。
    pub(super) async fn seed(date: NaiveDate) {
        let pool = crate::infra::database::get_connection();
        for (symbol, market_id, suspend_listing, change, volume) in ROWS {
            sqlx::query(
                r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol,
                                       stock_exchange_market_id, stock_industry_id, "SuspendListing")
                   VALUES ($1, $2, $1, $3, 24, $4)"#,
            )
            .bind(symbol)
            .bind(format!("測試{symbol}"))
            .bind(market_id)
            .bind(suspend_listing)
            .execute(pool)
            .await
            .expect("插入股票主檔");
            let change = Decimal::new(change, 0);
            let volume = Decimal::new(volume, 0);
            sqlx::query(
                r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "Change",
                                              "ChangeRange", "OpeningPrice", "HighestPrice",
                                              "LowestPrice", "TradingVolume", "TradeValue",
                                              "Transaction")
                   VALUES ($1, $2, 100 + $3, $3, $3, 100, 100 + $3, 100, $4, $4 * 100, 10)"#,
            )
            .bind(date)
            .bind(symbol)
            .bind(change)
            .bind(volume)
            .execute(pool)
            .await
            .expect("插入日報價");
        }
    }

    /// 刪除種入的日報價與股票主檔。
    pub(super) async fn cleanup(date: NaiveDate) {
        let pool = crate::infra::database::get_connection();
        let symbols: Vec<&str> = ROWS.iter().map(|row| row.0).collect();
        sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE "Date" = $1 AND stock_symbol = ANY($2)"#)
            .bind(date)
            .bind(&symbols)
            .execute(pool)
            .await
            .expect("清除日報價");
        sqlx::query("DELETE FROM stocks WHERE stock_symbol = ANY($1)")
            .bind(&symbols)
            .execute(pool)
            .await
            .expect("清除股票主檔");
    }
}
