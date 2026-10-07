//! CAGR 排行與個股 CAGR endpoints：參數驗證與資料庫語意整合測試。

use super::*;

/// M4 參數驗證一律在觸及資料庫之前完成，因此此測試不需要 PostgreSQL。
///
/// 最關鍵的是 `Y5`／`Y10` 搭配 `metric=price` 必須回 422：近十年每年皆有
/// 134–216 檔股票配股，長期間忽略配股的低估幅度顯著，不能默默回答。
#[tokio::test]
async fn cagr_ranking_rejects_invalid_parameters_before_any_query() {
    // Auth middleware 讀環境變數 DATA_API_KEY；測試環境沒設定時自行
    // 補一組（CI 以 --test-threads=1 執行，無資料競爭疑慮）。
    let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
        let generated = "cagr-param-test-key".to_owned();
        unsafe { std::env::set_var("DATA_API_KEY", &generated) };
        generated
    });
    for path in [
        // 期間、口徑、排序鍵的白名單。
        "/api/v1/market/cagr-ranking?period=Y8",
        "/api/v1/market/cagr-ranking?period=y1",
        "/api/v1/market/cagr-ranking?metric=cash",
        "/api/v1/market/cagr-ranking?sort=return",
        // 長期間不提供純價格口徑。
        "/api/v1/market/cagr-ranking?period=Y5&metric=price",
        "/api/v1/market/cagr-ranking?period=Y10&metric=price",
        // 市場、產業、關鍵字、分頁與日期。
        "/api/v1/market/cagr-ranking?market=emerging",
        "/api/v1/market/cagr-ranking?stock_industry_id=0",
        "/api/v1/market/cagr-ranking?limit=0",
        "/api/v1/market/cagr-ranking?limit=201",
        "/api/v1/market/cagr-ranking?date=2026-8-6",
        // 個股端點共用同一組解析器。
        "/api/v1/market/cagr-ranking/2330?metric=cash",
        "/api/v1/market/cagr-ranking/2330?date=2026-08-6",
    ] {
        let response = router()
            .oneshot(
                Request::get(path)
                    .header("Authorization", format!("Bearer {key}"))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("router should serve request");
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{path} 應回 422"
        );
    }
    // 短期間可用純價格口徑這條正向路徑會觸及資料庫，交由整合測試
    // （cagr_endpoints_db_semantics）驗證，此處刻意不發出該請求。
}

/// M4 兩個 endpoint 的真實資料庫語意整合測試（唯讀，不建立任何 fixture）。
///
/// 驗證：全市場名次不受篩選影響且遞增、資料不足項目 `rank` 為 null 且
/// 排在最後、金額欄位序列化為字串、涵蓋率分母正確，以及個股端點回傳
/// 全部八個期間並依期間長度排序。M3 排程尚未產出資料時安全跳過。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn cagr_endpoints_db_semantics() {
    dotenvy::dotenv().ok();
    if sqlx::query("SELECT 1")
        .execute(crate::infra::database::get_connection())
        .await
        .is_err()
    {
        println!("跳過 cagr_endpoints_db_semantics：無資料庫連接");
        return;
    }
    let key = std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
        let generated = "cagr-integration-test-key".to_owned();
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

    // 先以「必然無資料的基準日」跑一次完整查詢：即使排程尚未產出任何
    // 結果，這條路徑仍會把排行 SQL（CTE、視窗函式、三個篩選、分頁）
    // 送進 PostgreSQL，語法或欄位錯誤會立刻現形。
    for path in [
        "/api/v1/market/cagr-ranking?date=1990-01-03&period=Y1",
        "/api/v1/market/cagr-ranking?date=1990-01-03&period=M3&sort=total_return",
        "/api/v1/market/cagr-ranking?date=1990-01-03&period=Y3&metric=price&market=twse",
        "/api/v1/market/cagr-ranking?date=1990-01-03&metric=reinvested&stock_industry_id=24",
        "/api/v1/market/cagr-ranking?date=1990-01-03&keyword=%E5%8F%B0%E7%A9%8D&offset=10",
        "/api/v1/market/cagr-ranking?date=1990-01-03&include_incomplete=false&limit=200",
    ] {
        let (status, json) = get(path.to_owned()).await;
        assert_eq!(status, StatusCode::OK, "{path} 應能在真實 schema 執行");
        assert_eq!(json["items"], serde_json::json!([]));
        assert_eq!(json["total"], 0);
        assert_eq!(json["coverage"]["universe"], 0);
        assert_eq!(json["base_date"], serde_json::Value::Null);
        assert_eq!(json["coverage"]["coverage_ratio"], "0.0000");
        assert_eq!(json["summary"]["positive_ratio"], "0.0000");
    }

    // 自行寫入一組計算結果再驗證有資料時的回應。早期版本改為「排程尚未
    // 產出結果就跳過」，於是 CI 從未跑過表頭、名次、涵蓋統計與個股端點 ——
    // 那正是這兩個 endpoint 的主體。資料一律用假代號與 1990-01-02，
    // 結束時清除。
    cagr_seed::cleanup().await;
    cagr_seed::seed().await;

    let (status, json) =
        get("/api/v1/market/cagr-ranking?date=1990-01-02&period=Y1&limit=50".to_owned()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["period"], "Y1");
    assert_eq!(json["metric"], "total");
    assert_eq!(json["sort"], "cagr", "Y1 預設應以年化報酬率排序");
    assert_eq!(json["principal"], 10_000);
    assert_eq!(json["coverage"]["survivorship_note"], false);
    // 比率一律是字串（四位小數），不可是 JSON number。
    for pointer in ["/coverage/coverage_ratio", "/summary/positive_ratio"] {
        let value = json.pointer(pointer).expect("ratio 欄位應存在");
        assert!(value.is_string(), "{pointer} 必須序列化為字串");
    }

    // 表頭的期初日與年數取自可算項目，不得因為有資料不足的列就變成 null。
    assert_eq!(json["date"], "1990-01-02");
    assert_eq!(json["base_date"], "1989-01-03");
    assert!(json["years"].is_string());
    assert_eq!(json["coverage"]["universe"], 3);
    assert_eq!(json["coverage"]["counted"], 2);
    assert_eq!(json["coverage"]["incomplete"], 1);
    assert_eq!(json["total"], 3);

    let items = json["items"].as_array().expect("items array");
    assert_eq!(items.len(), 3);
    let mut previous_rank = 0_i64;
    let mut seen_incomplete = false;
    for item in items {
        match item["rank"].as_i64() {
            Some(rank) => {
                assert!(!seen_incomplete, "資料不足的項目必須排在所有可算項目之後");
                assert!(rank > previous_rank, "名次必須嚴格遞增");
                assert!(item["data_complete"].as_bool().unwrap_or(false));
                assert!(item["cagr_pct"].is_string(), "金額與比率必須是字串");
                assert!(item["end_shares"].is_string());
                previous_rank = rank;
            }
            None => {
                seen_incomplete = true;
                // 查得到但算不出來：列仍在，數值欄位為 null。
                assert!(item["cagr_pct"].is_null());
                assert!(item["end_value"].is_null());
                assert!(item["stock_symbol"].is_string());
            }
        }
    }

    // 全市場名次：套用產業篩選後，名次仍是未篩選的完整市場排名。
    let (status, filtered) = get(format!(
        "/api/v1/market/cagr-ranking?date=1990-01-02&period=Y1&stock_industry_id={}&limit=50",
        cagr_seed::INDUSTRY
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        filtered["coverage"], json["coverage"],
        "涵蓋統計不隨畫面篩選變動"
    );
    assert!(
        filtered["total"].as_i64() <= json["total"].as_i64(),
        "篩選後的 total 不可大於未篩選"
    );
    let filtered_ranks: Vec<Option<i64>> = filtered["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|item| item["rank"].as_i64())
        .collect();
    assert_eq!(
        filtered_ranks,
        items
            .iter()
            .map(|item| item["rank"].as_i64())
            .collect::<Vec<_>>(),
        "篩選後名次不得重編"
    );

    // 市場篩選對應到 stock_exchange_market_id：上櫃篩選不含這批上市假股票。
    let (status, tpex) =
        get("/api/v1/market/cagr-ranking?date=1990-01-02&period=Y1&market=tpex".to_owned()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tpex["total"], 0);

    // 關鍵字比對代號與名稱。
    let (status, keyword) = get(format!(
        "/api/v1/market/cagr-ranking?date=1990-01-02&period=Y1&keyword={}",
        cagr_seed::TOP_SYMBOL
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(keyword["total"], 1);

    // include_incomplete = false 時資料不足的列整列消失。
    let (status, complete_only) = get(
        "/api/v1/market/cagr-ranking?date=1990-01-02&period=Y1&include_incomplete=false".to_owned(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(complete_only["total"], 2);
    assert!(
        complete_only["items"]
            .as_array()
            .expect("items array")
            .iter()
            .all(|item| item["rank"].is_i64())
    );

    // Y10 + price 由 handler 在任何 SQL 之前擋下。
    let (status, _) = get("/api/v1/market/cagr-ranking?period=Y10&metric=price".to_owned()).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Y10 的揭露旗標必須為 true，且長期間不提供純價格口徑（欄位為 null）。
    let (status, json) =
        get("/api/v1/market/cagr-ranking?date=1990-01-02&period=Y10&limit=1".to_owned()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["coverage"]["survivorship_note"], true);

    // 個股端點：未知代號 404；已知代號回傳全部八個期間且由短至長。
    let (status, _) = get("/api/v1/market/cagr-ranking/NO_SUCH_SYMBOL".to_owned()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 代號存在但該基準日沒有計算結果 —— 與「代號打錯」同為 404，但走的是
    // 另一條分支。
    let (status, _) = get(format!(
        "/api/v1/market/cagr-ranking/{}?date=1990-01-03",
        cagr_seed::TOP_SYMBOL
    ))
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, json) = get(format!(
        "/api/v1/market/cagr-ranking/{}?date=1990-01-02",
        cagr_seed::TOP_SYMBOL
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["stock_symbol"], cagr_seed::TOP_SYMBOL);
    assert_eq!(json["principal"], 10_000);
    let periods: Vec<&str> = json["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|item| item["period"].as_str().expect("period string"))
        .collect();
    assert_eq!(
        periods,
        vec!["M3", "M6", "Y1", "Y1H", "Y2", "Y3", "Y5", "Y7", "Y10"],
        "個股端點必須回傳全部九個期間且依期間長度排序"
    );
    assert!(
        json["items"]
            .as_array()
            .expect("items array")
            .iter()
            .all(|item| item["rank"].is_null()),
        "個股端點的項目不應有名次"
    );

    // 指定 price 口徑時，長期間該口徑為 null 但列仍在。
    let (status, price) = get(format!(
        "/api/v1/market/cagr-ranking/{}?date=1990-01-02&metric=price",
        cagr_seed::TOP_SYMBOL
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    let long_period = price["items"]
        .as_array()
        .expect("items array")
        .iter()
        .find(|item| item["period"] == "Y10")
        .expect("Y10 應在清單中");
    assert!(long_period["cagr_pct"].is_null(), "長期間不提供純價格口徑");

    cagr_seed::cleanup().await;
}

/// `cagr_endpoints_db_semantics` 專用的測試資料。
///
/// 代號一律以 `79979` 開頭（真實市場不存在），基準日固定 1990-01-02，
/// 遠早於本功能上線，不會與正式資料互相干擾。
///
/// 不加 `#[cfg(feature = "integration-tests")]`：使用它的測試函式在未開
/// feature 時只是被標記為 `ignore`，本體仍要通過編譯。
mod cagr_seed {
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    use crate::domain::performance::{
        CagrPeriod, CagrRepository, StockCagr, entity::SimulationOutcome,
    };
    use crate::infra::database;
    use crate::infra::database::repository::performance::PgCagrRepository;

    /// 名次第一的假代號，同時用於個股端點與關鍵字篩選。
    pub(super) const TOP_SYMBOL: &str = "79979E1";
    const SECOND_SYMBOL: &str = "79979E2";
    const INCOMPLETE_SYMBOL: &str = "79979E3";
    /// 水泥工業，`stock_industry.sql` 已預先建立。
    pub(super) const INDUSTRY: i32 = 1;
    /// 上市（twse）；`cagr_market_id` 把 "twse" 對應到 2。
    const MARKET: i32 = 2;

    fn symbols() -> Vec<String> {
        vec![
            TOP_SYMBOL.to_string(),
            SECOND_SYMBOL.to_string(),
            INCOMPLETE_SYMBOL.to_string(),
        ]
    }

    fn base_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(1990, 1, 2).expect("測試日期應合法")
    }

    fn record(symbol: &str, period: CagrPeriod, cagr: rust_decimal::Decimal) -> StockCagr {
        let outcome = SimulationOutcome {
            end_shares: dec!(100.5),
            cash_received: dec!(250.0),
            end_value: dec!(12000.0),
            total_return_pct: dec!(20.0),
            cagr_pct: cagr,
        };
        StockCagr {
            date: base_date(),
            stock_symbol: symbol.to_string(),
            period,
            base_date: NaiveDate::from_ymd_opt(1989, 1, 3),
            base_price: Some(dec!(100.0)),
            end_price: Some(dec!(119.4)),
            years: Some(dec!(1.0)),
            // 與計算層一致：長期間不提供純價格口徑。
            price: period.supports_price_metric().then_some(outcome),
            total: Some(outcome),
            reinvested: Some(outcome),
            first_quote_date: NaiveDate::from_ymd_opt(1988, 1, 4),
            shortfall_days: Some(0),
            data_complete: true,
            has_anomaly: false,
            dividend_events: 2,
        }
    }

    fn incomplete(symbol: &str, period: CagrPeriod) -> StockCagr {
        StockCagr {
            date: base_date(),
            stock_symbol: symbol.to_string(),
            period,
            base_date: None,
            base_price: None,
            end_price: None,
            years: None,
            price: None,
            total: None,
            reinvested: None,
            first_quote_date: NaiveDate::from_ymd_opt(1989, 6, 1),
            shortfall_days: None,
            data_complete: false,
            has_anomaly: true,
            dividend_events: 0,
        }
    }

    pub(super) async fn seed() {
        for (index, symbol) in symbols().iter().enumerate() {
            let _ = sqlx::query(
                r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_industry_id,
                                       stock_exchange_market_id, "SuspendListing")
                   VALUES ($1, $2, $1, $3, $4, false)
                   ON CONFLICT (stock_symbol) DO UPDATE
                       SET "Name" = excluded."Name",
                           stock_industry_id = excluded.stock_industry_id,
                           stock_exchange_market_id = excluded.stock_exchange_market_id"#,
            )
            .bind(symbol)
            .bind(format!("測試股{}", index + 1))
            .bind(INDUSTRY)
            .bind(MARKET)
            .execute(database::get_connection())
            .await;
        }

        // 名次第一的個股寫滿八個期間，供個股端點驗證排序；
        // 另外兩檔只寫 Y1，構成「可算 2 檔 + 資料不足 1 檔」的母體。
        let mut records: Vec<StockCagr> = CagrPeriod::ALL
            .into_iter()
            .map(|period| record(TOP_SYMBOL, period, dec!(30)))
            .collect();
        records.push(record(SECOND_SYMBOL, CagrPeriod::Y1, dec!(10)));
        records.push(incomplete(INCOMPLETE_SYMBOL, CagrPeriod::Y1));

        PgCagrRepository::new()
            .save_batch(&records)
            .await
            .expect("寫入測試用 CAGR 結果");
    }

    pub(super) async fn cleanup() {
        let _ = sqlx::query("DELETE FROM stock_cagr WHERE stock_symbol = ANY($1)")
            .bind(symbols())
            .execute(database::get_connection())
            .await;
        let _ = sqlx::query("DELETE FROM stocks WHERE stock_symbol = ANY($1)")
            .bind(symbols())
            .execute(database::get_connection())
            .await;
    }
}
