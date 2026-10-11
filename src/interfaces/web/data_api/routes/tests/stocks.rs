//! `/stocks/*` 報價與基本面 endpoints 的資料庫語意整合測試。
//!
//! CI 的測試庫只有 schema 沒有資料，`phases` 那組測試在 CI 只跑得到 404／422 分支；
//! 這裡種入一檔假代號的完整資料，讓查詢結果與欄位轉換真的被驗證到。

use std::collections::HashMap;

use rust_decimal::Decimal;

use super::*;
use crate::infra::cache::{RealtimeSnapshot, SHARE};

/// 假代號；跑完會清掉所有種入的列。
const SYMBOL: &str = "79979DA";

/// 回傳可用的 API 金鑰；測試環境沒設定時自行補一組（`--test-threads=1` 下無競爭）。
fn api_key() -> String {
    std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
        let generated = "stocks-integration-test-key".to_owned();
        unsafe { std::env::set_var("DATA_API_KEY", &generated) };
        generated
    })
}

/// 以真實 router 發出帶金鑰的 GET，回傳狀態碼與 JSON body。
async fn get(path: &str) -> (StatusCode, serde_json::Value) {
    let response = router()
        .oneshot(
            Request::get(path)
                .header("Authorization", format!("Bearer {}", api_key()))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    (
        status,
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
    )
}

/// 資料庫可用時回傳連線池，否則印出跳過訊息。
async fn pool_or_skip(test: &str) -> Option<&'static sqlx::PgPool> {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 {test}：無資料庫連接");
        return None;
    }
    Some(pool)
}

/// 刪除假代號在各表的資料。
async fn cleanup(pool: &sqlx::PgPool) {
    for sql in [
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
        "DELETE FROM last_daily_quotes WHERE stock_symbol = $1",
        "DELETE FROM quote_history_record WHERE security_code = $1",
        r#"DELETE FROM "Revenue" WHERE "SecurityCode" = $1"#,
        "DELETE FROM financial_statement WHERE security_code = $1",
        "DELETE FROM dividend WHERE security_code = $1",
        "DELETE FROM estimate WHERE security_code = $1",
        "DELETE FROM income_statement WHERE stock_symbol = $1",
        "DELETE FROM balance_sheet WHERE stock_symbol = $1",
        "DELETE FROM cash_flow_statement WHERE stock_symbol = $1",
        "DELETE FROM stocks WHERE stock_symbol = $1",
    ] {
        let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
    }
}

/// 依序執行種入資料的 SQL（全部只綁 `$1 = SYMBOL`）。
async fn seed(pool: &sqlx::PgPool, statements: &[&'static str]) {
    for &sql in statements {
        sqlx::query(sql)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("種入測試資料失敗：{error}\n{sql}"));
    }
}

/// 股票主檔一列；兩組測試共用。
const SEED_STOCK: &str = r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id, "SuspendListing", last_one_eps, last_four_eps, net_asset_value_per_share, return_on_equity, weight, issued_share)
    VALUES ($1, '端點測試股', $1, 2, 1, false, 1.25, 5.5, 30.1, 18.2, 0.01, 1000000)"#;

/// 搜尋、最新報價、歷史日線、完整基本面與近即時快照。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn stock_quote_endpoints_db_semantics() {
    let Some(pool) = pool_or_skip("stock_quote_endpoints_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool, &[SEED_STOCK]).await;

    // 主檔存在但還沒有任何報價：latest-quote 的 quote 是 null、price-history 是空陣列。
    let (status, json) = get(&format!("/api/v1/stocks/{SYMBOL}/latest-quote")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["stock"]["name"], "端點測試股");
    assert!(json["quote"].is_null(), "{json}");
    let (status, json) = get(&format!("/api/v1/stocks/{SYMBOL}/price-history")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["quotes"], serde_json::json!([]));

    seed(
        pool,
        &[
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, year, month, day, "OpeningPrice", "HighestPrice", "LowestPrice", "ClosingPrice", "Change", "ChangeRange", "TradingVolume", "Transaction", "TradeValue", "MovingAverage5", "PriceEarningRatio", "price-to-book_ratio")
               VALUES ('2026-04-29', $1, 2026, 4, 29, 100, 102, 99, 101, 1, 1, 5000, 300, 505000, 100.5, 15.2, 1.8),
                      ('2026-04-30', $1, 2026, 4, 30, 101, 104, 100, 103.5, 2.5, 2.48, 6000, 320, 621000, 101.2, 15.6, 1.85)"#,
            r#"INSERT INTO last_daily_quotes (date, stock_symbol, opening_price, highest_price, lowest_price, closing_price, change, change_range, trading_volume, transaction, trade_value, moving_average_5, price_earning_ratio)
               VALUES ('2026-04-30', $1, 101, 104, 100, 103.5, 2.5, 2.48, 6000, 320, 621000, 101.2, 15.6)"#,
            r#"INSERT INTO quote_history_record (security_code, maximum_price, maximum_price_date_on, minimum_price, minimum_price_date_on)
               VALUES ($1, 120, '2025-07-01', 80, '2024-01-02')"#,
        ],
    )
    .await;

    // 參數不合法 → 422；未知代號 → 404。
    for path in [
        "/api/v1/stocks/search?query=".to_owned(),
        format!("/api/v1/stocks/search?query={SYMBOL}&limit=51"),
        format!("/api/v1/stocks/{SYMBOL}/price-history?limit=0"),
        format!("/api/v1/stocks/{SYMBOL}/price-history?from=2026-05-01&to=2026-04-01"),
    ] {
        assert_eq!(
            get(&path).await.0,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{path}"
        );
    }
    for path in [
        "/api/v1/stocks/79979ZZ/latest-quote",
        "/api/v1/stocks/79979ZZ/price-history",
        "/api/v1/stocks/79979ZZ/profile",
    ] {
        assert_eq!(get(path).await.0, StatusCode::NOT_FOUND, "{path}");
    }

    let (status, json) = get(&format!("/api/v1/stocks/search?query={SYMBOL}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["stocks"][0]["stock_symbol"], SYMBOL);
    assert_eq!(json["stocks"][0]["stock_exchange_market_id"], 2);

    let (status, json) = get(&format!("/api/v1/stocks/{SYMBOL}/latest-quote")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["quote"]["date"], "2026-04-30");
    assert_eq!(json["quote"]["closing_price"], 103.5);
    assert_eq!(json["quote"]["change_range"], 2.48);

    // 新到舊；limit 與日期區間都要生效。
    let (status, json) = get(&format!("/api/v1/stocks/{SYMBOL}/price-history")).await;
    assert_eq!(status, StatusCode::OK);
    let quotes = json["quotes"].as_array().expect("quotes array");
    assert_eq!(quotes.len(), 2);
    assert_eq!(quotes[0]["date"], "2026-04-30");
    assert_eq!(quotes[0]["price_to_book_ratio"], 1.85);
    assert_eq!(quotes[1]["closing_price"], 101.0);
    let (_, json) = get(&format!("/api/v1/stocks/{SYMBOL}/price-history?limit=1")).await;
    assert_eq!(json["quotes"].as_array().map(Vec::len), Some(1));
    let (_, json) = get(&format!(
        "/api/v1/stocks/{SYMBOL}/price-history?from=2026-04-29&to=2026-04-29"
    ))
    .await;
    assert_eq!(json["quotes"][0]["date"], "2026-04-29");
    assert_eq!(json["quotes"].as_array().map(Vec::len), Some(1));

    let (status, json) = get(&format!("/api/v1/stocks/{SYMBOL}/profile")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["stock"]["security_code"], SYMBOL);
    assert_eq!(json["quote"]["closing_price"], 103.5);
    assert_eq!(json["last_four_eps"], 5.5);
    assert_eq!(json["return_on_equity"], 18.2);
    assert_eq!(json["issued_share"], 1_000_000.0);
    assert_eq!(json["history"]["maximum_price"], 120.0);
    assert_eq!(json["history"]["maximum_price_date_on"], "2025-07-01");
    assert_eq!(json["history"]["minimum_price_date_on"], "2024-01-02");

    // 近即時快照：只在快取為空時測，避免動到別的測試或盤中留下的快照。
    if SHARE.stock_snapshots_are_empty() {
        let (status, _) = get(&format!("/api/v1/stocks/{SYMBOL}/realtime-snapshot")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let mut snapshot = RealtimeSnapshot::new(SYMBOL.to_owned(), Decimal::new(1035, 1));
        snapshot.name = "端點測試股".to_owned();
        snapshot.last_close = Decimal::new(101, 0);
        snapshot.change = Decimal::new(25, 1);
        snapshot.volume = Decimal::new(6000, 0);
        snapshot.source_site = "test".to_owned();
        SHARE.set_stock_snapshots(HashMap::from([(SYMBOL.to_owned(), snapshot)]));

        let (status, json) = get(&format!("/api/v1/stocks/{SYMBOL}/realtime-snapshot")).await;
        let (missing, _) = get("/api/v1/stocks/79979ZZ/realtime-snapshot").await;
        SHARE.clear_stock_snapshots();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["price"], 103.5);
        assert_eq!(json["last_close"], 101.0);
        assert_eq!(json["volume_lots"], 6000.0);
        assert_eq!(json["source_site"], "test");
        assert_eq!(missing, StatusCode::NOT_FOUND);
    }

    cleanup(pool).await;
}

/// 月營收、財報、股利與估值。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn stock_fundamental_endpoints_db_semantics() {
    let Some(pool) = pool_or_skip("stock_fundamental_endpoints_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    seed(
        pool,
        &[
            SEED_STOCK,
            r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly", "LastMonth", "LastYearThisMonth", "MonthlyAccumulated", "LastYearMonthlyAccumulated", "ComparedWithLastMonth", "ComparedWithLastYearSameMonth", "AccumulatedComparedWithLastYear", avg_price, lowest_price, highest_price)
               VALUES ($1, $1, 202602, 900, 1000, 800, 1900, 1600, -10, 12.5, 18.75, 98, 95, 101),
                      ($1, $1, 202603, 1100, 900, 1000, 3000, 2600, 22.22, 10, 15.38, 100, 97, 104)"#,
            r#"INSERT INTO financial_statement (security_code, "year", quarter, gross_profit, operating_profit_margin, "pre-tax_income", net_income, net_asset_value_per_share, sales_per_share, earnings_per_share, profit_before_tax, return_on_equity, return_on_assets)
               VALUES ($1, 2025, 'Q4', 40, 20, 21, 18, 30, 12, 1.5, 1.8, 5, 3),
                      ($1, 2025, '', 41, 21, 22, 19, 30, 48, 5.5, 6.6, 19, 11),
                      ($1, 2026, 'Q1', 42, 22, 23, 20, 31, 13, 1.6, 1.9, 5.2, 3.1)"#,
            r#"INSERT INTO dividend (security_code, "year", year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", payout_ratio, "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2)
               VALUES ($1, 2025, 2024, '', 3, 0.5, 3.5, 63.6, '2025-07-10', '尚未公布', '2025-08-08', '-'),
                      ($1, 2026, 2025, 'Q1', 1.2, 0, 1.2, 80, '2026-04-15', '-', '2026-05-12', '-')"#,
            r#"INSERT INTO estimate (security_code, date, closing_price, percentage, year_count, cheap, fair, expensive, price_cheap, price_fair, price_expensive)
               VALUES ($1, '2026-04-29', 101, 95, 10, 90, 105, 120, 88, 104, 118),
                      ($1, '2026-04-30', 103.5, 98.57, 10, 92, 106, 121, 90, 105, 119)"#,
        ],
    )
    .await;

    for path in [
        format!("/api/v1/stocks/{SYMBOL}/monthly-revenues?to=2026-13"),
        format!("/api/v1/stocks/{SYMBOL}/financial-statements?limit=41"),
        format!("/api/v1/stocks/{SYMBOL}/dividends?to_year=1989"),
        format!("/api/v1/stocks/{SYMBOL}/valuation?date=2026-02-30"),
    ] {
        assert_eq!(
            get(&path).await.0,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{path}"
        );
    }

    // 月營收：新到舊，月份轉 YYYY-MM，區間過濾生效。
    let (status, json) = get(&format!("/api/v1/stocks/{SYMBOL}/monthly-revenues")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data_as_of"], "2026-03");
    assert_eq!(json["revenues"][0]["month"], "2026-03");
    assert_eq!(json["revenues"][0]["monthly_revenue"], 1100.0);
    assert_eq!(json["revenues"][0]["month_over_month_percent"], 22.22);
    assert_eq!(json["revenues"][1]["month_over_month_percent"], -10.0);
    let (_, json) = get(&format!(
        "/api/v1/stocks/{SYMBOL}/monthly-revenues?from=2026-02&to=2026-02"
    ))
    .await;
    assert_eq!(json["revenues"].as_array().map(Vec::len), Some(1));
    assert_eq!(json["revenues"][0]["highest_price"], 101.0);

    // 財報：預設只有季報；年度的空字串對外是 A；all 時同一年度年報排在 Q4 前。
    let (status, json) = get(&format!("/api/v1/stocks/{SYMBOL}/financial-statements")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data_as_of"], "2026-Q1");
    let statements = json["statements"].as_array().expect("statements array");
    assert_eq!(statements.len(), 2);
    assert_eq!(statements[0]["earnings_per_share"], 1.6);
    assert_eq!(statements[1]["quarter"], "Q4");
    let (_, json) = get(&format!(
        "/api/v1/stocks/{SYMBOL}/financial-statements?period_type=annual"
    ))
    .await;
    assert_eq!(json["data_as_of"], "2025-A");
    assert_eq!(json["statements"][0]["sales_per_share"], 48.0);
    let (_, json) = get(&format!(
        "/api/v1/stocks/{SYMBOL}/financial-statements?period_type=all"
    ))
    .await;
    let quarters: Vec<_> = json["statements"]
        .as_array()
        .expect("statements array")
        .iter()
        .map(|statement| statement["quarter"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(quarters, ["Q1", "A", "Q4"]);

    // 股利：依所屬年度新到舊；不合法的日期字串輸出 null。
    let (status, json) = get(&format!("/api/v1/stocks/{SYMBOL}/dividends")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data_as_of"], "2025-Q1");
    let dividends = json["dividends"].as_array().expect("dividends array");
    assert_eq!(dividends.len(), 2);
    assert_eq!(dividends[0]["paid_year"], 2026);
    assert_eq!(dividends[1]["quarter"], "A");
    assert_eq!(dividends[1]["total_dividend"], 3.5);
    assert_eq!(dividends[1]["ex_dividend_date"], "2025-07-10");
    assert!(dividends[1]["ex_rights_date"].is_null(), "{json}");
    assert!(dividends[1]["stock_payable_date"].is_null(), "{json}");
    let (_, json) = get(&format!(
        "/api/v1/stocks/{SYMBOL}/dividends?from_year=2024&to_year=2024"
    ))
    .await;
    assert_eq!(json["dividends"].as_array().map(Vec::len), Some(1));
    assert_eq!(json["dividends"][0]["dividend_year"], 2024);

    // 估值：不帶日期取最新一筆；指定日期往前找；視窗內沒有資料時 valuation 是 null。
    let (status, json) = get(&format!("/api/v1/stocks/{SYMBOL}/valuation")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data_as_of"], "2026-04-30");
    assert_eq!(json["valuation"]["closing_price"], 103.5);
    assert_eq!(json["valuation"]["valuation_band"], "fair_valued");
    let (_, json) = get(&format!(
        "/api/v1/stocks/{SYMBOL}/valuation?date=2026-04-29"
    ))
    .await;
    assert_eq!(json["valuation"]["date"], "2026-04-29");
    assert_eq!(json["valuation"]["price_fair"], 104.0);
    let (status, json) = get(&format!(
        "/api/v1/stocks/{SYMBOL}/valuation?date=2026-01-01"
    ))
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["valuation"].is_null(), "{json}");
    assert!(json["data_as_of"].is_null(), "{json}");

    cleanup(pool).await;
}

/// 三大報表：Yahoo 與 BigGo 同期並存時取 Yahoo、只有 BigGo 的期間照樣回傳、
/// 年度模式的資產負債表取第 4 季季末，以及 422／404 與尚無資料的空清單。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn stock_financial_report_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("stock_financial_report_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool, &[SEED_STOCK]).await;

    // 主檔存在但還沒有報表：三個清單都是空陣列、data_as_of 是 null。
    let path = format!("/api/v1/stocks/{SYMBOL}/financial-reports");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["period_type"], "quarterly");
    assert_eq!(json["amount_unit"], "thousand_twd");
    assert!(json["data_as_of"].is_null(), "{json}");
    for list in [
        "income_statements",
        "balance_sheets",
        "cash_flow_statements",
    ] {
        assert_eq!(json[list], serde_json::json!([]), "{list}");
    }

    seed(
        pool,
        &[
            // 2020 四季與年度來自 Yahoo；2020-Q4 另有一筆 BigGo（應被 Yahoo 蓋過）；
            // 2019-Q4 只有 BigGo（BigGo 不提供的欄位為 NULL）。
            r#"INSERT INTO income_statement (stock_symbol, fiscal_year, period_type, quarter, source, revenue, gross_profit, operating_profit, net_income, owner_parent_profit, eps, bps)
               VALUES ($1, 2020, 'single', 'Q1', 'yahoo', 1000, 400, 200, 150, 140, 1.10, 20.50),
                      ($1, 2020, 'single', 'Q2', 'yahoo', 1100, 450, 220, 160, 150, 1.20, 21.00),
                      ($1, 2020, 'single', 'Q3', 'yahoo', 1200, 500, 240, 170, 160, 1.30, 21.50),
                      ($1, 2020, 'single', 'Q4', 'yahoo', 1300, 550, -10, -20, -25, -0.25, 21.40),
                      ($1, 2020, 'single', 'Q4', 'biggo', 9999, NULL, 9999, 9999, NULL, 9.99, NULL),
                      ($1, 2019, 'single', 'Q4', 'biggo', 900, NULL, 100, 80, NULL, 0.80, NULL),
                      ($1, 2020, 'annual', 'A', 'yahoo', 4600, 1900, 650, 460, 425, 3.35, 21.40)"#,
            r#"INSERT INTO balance_sheet (stock_symbol, fiscal_year, period_type, quarter, source, total_assets, total_liabilities, equity, book_value_per_share)
               VALUES ($1, 2020, 'single', 'Q3', 'yahoo', 50000, 20000, 30000, 21.50),
                      ($1, 2020, 'single', 'Q4', 'yahoo', 52000, 21000, 31000, 21.40),
                      ($1, 2019, 'single', 'Q4', 'biggo', 48000, 19000, 29000, NULL)"#,
            r#"INSERT INTO cash_flow_statement (stock_symbol, fiscal_year, period_type, quarter, source, operating_cash_flow, investing_cash_flow, financing_cash_flow, free_cash_flow, net_cash_flow)
               VALUES ($1, 2020, 'single', 'Q4', 'yahoo', 300, -120, -50, 180, 130),
                      ($1, 2020, 'annual', 'A', 'yahoo', 900, -400, -200, 500, 300)"#,
        ],
    )
    .await;

    // 單季：期間由新到舊，limit 分別套用在三張報表；同期 BigGo 被 Yahoo 取代。
    let (status, json) = get(&format!("{path}?limit=3")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data_as_of"], "2020-Q4");
    let income = json["income_statements"].as_array().expect("損益表陣列");
    let periods: Vec<_> = income
        .iter()
        .map(|row| {
            format!(
                "{}-{}",
                row["fiscal_year"],
                row["quarter"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(periods, ["2020-Q4", "2020-Q3", "2020-Q2"]);
    assert_eq!(income[0]["source"], "yahoo");
    assert_eq!(income[0]["revenue"], 1300);
    assert_eq!(income[0]["operating_profit"], -10);
    assert_eq!(income[0]["eps"], -0.25);
    assert!(income[0]["selling_expenses"].is_null(), "{json}");
    assert!(income[0]["updated_at"].is_string());
    assert_eq!(json["balance_sheets"].as_array().map(Vec::len), Some(3));
    assert_eq!(json["balance_sheets"][0]["book_value_per_share"], 21.4);
    assert_eq!(
        json["cash_flow_statements"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(json["cash_flow_statements"][0]["free_cash_flow"], 180);

    // 不限期數時，只有 BigGo 的 2019-Q4 也會出現，並標示來源。
    let (_, json) = get(&format!("{path}?limit=40")).await;
    let income = json["income_statements"].as_array().expect("損益表陣列");
    assert_eq!(income.len(), 5);
    assert_eq!(income[4]["fiscal_year"], 2019);
    assert_eq!(income[4]["source"], "biggo");
    assert!(income[4]["gross_profit"].is_null(), "{json}");
    assert_eq!(json["balance_sheets"][2]["source"], "biggo");
    assert!(json["balance_sheets"][2]["book_value_per_share"].is_null());

    // 年度：損益與現金流量取 A；資產負債表取各年第 4 季季末。
    let (status, json) = get(&format!("{path}?period_type=annual")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["period_type"], "annual");
    assert_eq!(json["data_as_of"], "2020-A");
    assert_eq!(json["income_statements"].as_array().map(Vec::len), Some(1));
    assert_eq!(json["income_statements"][0]["eps"], 3.35);
    let balance: Vec<_> = json["balance_sheets"]
        .as_array()
        .expect("資產負債表陣列")
        .iter()
        .map(|row| {
            format!(
                "{}-{}",
                row["fiscal_year"],
                row["quarter"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(balance, ["2020-Q4", "2019-Q4"]);
    assert_eq!(json["cash_flow_statements"][0]["quarter"], "A");
    assert_eq!(json["cash_flow_statements"][0]["net_cash_flow"], 300);

    // 參數錯誤 422、未知代號 404。
    for query in ["period_type=all", "limit=0", "limit=41"] {
        let (status, _) = get(&format!("{path}?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }
    let (status, _) = get("/api/v1/stocks/79979ZZ/financial-reports").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    cleanup(pool).await;
}

/// 財務體質：兩個年度的 F 分數、Yahoo 優先於 BigGo、指定年度缺前一年時只評 3 項，
/// 以及尚無年度財報、422 與 404。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn stock_financial_health_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("stock_financial_health_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool, &[SEED_STOCK]).await;

    // 主檔存在但沒有年度財報：年度為 null、9 項都無法判定。
    let path = format!("/api/v1/stocks/{SYMBOL}/financial-health");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["fiscal_year"].is_null(), "{json}");
    assert!(json["compared_with"].is_null(), "{json}");
    assert_eq!(
        (json["score"].clone(), json["evaluated"].clone()),
        (0.into(), 0.into())
    );
    assert_eq!(json["max_score"], 9);
    let criteria = json["criteria"].as_array().expect("指標陣列");
    assert_eq!(criteria.len(), 9);
    assert!(criteria.iter().all(|c| c["passed"].is_null()), "{json}");

    seed(
        pool,
        &[
            // 2020 年 Yahoo 另有一筆 BigGo（淨利為負，若被選到資產報酬率會變負）。
            r#"INSERT INTO income_statement (stock_symbol, fiscal_year, period_type, quarter, source, revenue, gross_profit, net_income)
               VALUES ($1, 2020, 'annual', 'A', 'yahoo', 4600, 1900, 460),
                      ($1, 2020, 'annual', 'A', 'biggo', 4600, 1900, -9999),
                      ($1, 2019, 'annual', 'A', 'yahoo', 4000, 1500, 300),
                      ($1, 2020, 'single', 'Q4', 'yahoo', 1300, 550, 120)"#,
            r#"INSERT INTO cash_flow_statement (stock_symbol, fiscal_year, period_type, quarter, source, operating_cash_flow)
               VALUES ($1, 2020, 'annual', 'A', 'yahoo', 900),
                      ($1, 2019, 'annual', 'A', 'yahoo', 200)"#,
            r#"INSERT INTO balance_sheet (stock_symbol, fiscal_year, period_type, quarter, source, total_assets, current_assets, current_liabilities, non_current_liabilities, share_capital)
               VALUES ($1, 2020, 'single', 'Q3', 'yahoo', 99999, 1, 99999, 99999, 99999),
                      ($1, 2020, 'single', 'Q4', 'yahoo', 52000, 20000, 10000, 11000, 1000),
                      ($1, 2019, 'single', 'Q4', 'yahoo', 48000, 15000, 9000, 10000, 1000)"#,
        ],
    )
    .await;

    // 預設取最新年度 2020、比較 2019：只有長期負債比上升，8／9。
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["fiscal_year"], 2020);
    assert_eq!(json["compared_with"], 2019);
    assert_eq!(json["score"], 8, "{json}");
    assert_eq!(json["evaluated"], 9);
    let criteria = json["criteria"].as_array().expect("指標陣列");
    let failed: Vec<_> = criteria
        .iter()
        .filter(|c| c["passed"] == false)
        .map(|c| c["key"].as_str().unwrap())
        .collect();
    assert_eq!(failed, ["leverage_not_increased"]);
    assert_eq!(criteria[0]["key"], "roa_positive");
    assert_eq!(criteria[0]["value"], 0.0088);
    assert_eq!(criteria[3]["value"], 900.0);
    assert_eq!(criteria[3]["baseline"], 460.0);
    assert_eq!(criteria[3]["unit"], "thousand_twd");
    assert_eq!(criteria[5]["key"], "current_ratio_improved");
    assert_eq!(criteria[5]["value"], 2.0);

    // 指定 2019：沒有 2018 的資料，只能判定不需比較的 3 項；盈餘品質不及格（200 < 300）。
    let (status, json) = get(&format!("{path}?fiscal_year=2019")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["compared_with"], 2018);
    assert_eq!(
        (json["score"].clone(), json["evaluated"].clone()),
        (2.into(), 3.into())
    );

    for query in ["fiscal_year=1999", "fiscal_year=2101"] {
        let (status, _) = get(&format!("{path}?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }
    let (status, _) = get("/api/v1/stocks/79979ZZ/financial-health").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    cleanup(pool).await;
}
