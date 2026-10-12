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
pub(super) async fn get(path: &str) -> (StatusCode, serde_json::Value) {
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
pub(super) async fn pool_or_skip(test: &str) -> Option<&'static sqlx::PgPool> {
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
        "DELETE FROM financial_health WHERE stock_symbol = $1",
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

/// 條件選股的財務體質條件：新鮮的 F 分數、毛利率、負債比與自由現金流參與篩選與輸出；
/// 評分年度過舊時 F 分數轉成 null、不再符合 F 分數條件，但保留評分年度。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn stock_screen_financial_health_filters() {
    use chrono::Datelike;

    let Some(pool) = pool_or_skip("stock_screen_financial_health_filters").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool, &[SEED_STOCK]).await;
    // 新鮮度以查詢當天判斷，期間只能由今天推算；列屬於假代號，結束時清除。
    let today = chrono::Local::now().date_naive();
    let quarter = i16::try_from(today.month0() / 3 + 1).expect("季別 1–4");
    sqlx::query(
        "INSERT INTO financial_health (stock_symbol, f_score_year, f_score, f_score_evaluated, quarter_year, quarter, gross_margin_percent, debt_ratio_percent, free_cash_flow_ttm) VALUES ($1, $2, 8, 9, $3, $4, 42.5, 35.25, 1200)",
    )
    .bind(SYMBOL)
    .bind(today.year() - 1)
    .bind(today.year())
    .bind(quarter)
    .execute(pool)
    .await
    .expect("種入 financial_health");

    let listed = |json: &serde_json::Value| {
        json["stocks"]
            .as_array()
            .expect("stocks 陣列")
            .iter()
            .find(|stock| stock["stock_symbol"] == SYMBOL)
            .cloned()
    };
    let base = "/api/v1/stocks/screen?industry_id=1&limit=50";

    let (status, json) = get(&format!(
        "{base}&min_f_score=7&min_gross_margin_percent=40&max_debt_ratio_percent=50&positive_free_cash_flow=true&sort_by=f_score&sort_order=desc"
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let stock = listed(&json).expect("符合條件的假代號應出現");
    assert_eq!(stock["f_score"], 8);
    assert_eq!(stock["f_score_evaluated"], 9);
    assert_eq!(stock["f_score_year"], today.year() - 1);
    assert_eq!(stock["gross_margin_percent"], 42.5);
    assert_eq!(stock["debt_ratio_percent"], 35.25);
    assert_eq!(stock["free_cash_flow_ttm"], 1200);
    assert_eq!(
        stock["health_period"],
        format!("{}-Q{quarter}", today.year())
    );

    for query in [
        "min_f_score=9",
        "min_gross_margin_percent=50",
        "max_debt_ratio_percent=30",
    ] {
        let (status, json) = get(&format!("{base}&{query}")).await;
        assert_eq!(status, StatusCode::OK, "{query}");
        assert!(listed(&json).is_none(), "{query} 應排除假代號");
    }
    for query in [
        "min_f_score=10",
        "max_debt_ratio_percent=-1",
        "sort_by=f_scores",
    ] {
        let (status, _) = get(&format!("{base}&{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }

    // 評分年度落後 3 年：F 分數過期轉成 null、不符合 F 分數條件，評分年度照樣回傳。
    sqlx::query("UPDATE financial_health SET f_score_year = $2 WHERE stock_symbol = $1")
        .bind(SYMBOL)
        .bind(today.year() - 3)
        .execute(pool)
        .await
        .expect("改成過期的評分年度");
    let (_, json) = get(&format!("{base}&min_f_score=0")).await;
    assert!(listed(&json).is_none(), "過期的 F 分數不應符合條件");
    let (_, json) = get(&format!("{base}&positive_free_cash_flow=true")).await;
    let stock = listed(&json).expect("自由現金流仍新鮮，應出現");
    assert!(stock["f_score"].is_null(), "{stock}");
    assert_eq!(stock["f_score_year"], today.year() - 3);

    cleanup(pool).await;
}

/// 條件選股的營收動能：連續年增月數遇到年增 ≤ 0 或月份斷層就停止，
/// 近 3 月累計年增率以三個月合計營收對去年同期計算，三個月不相連時為 null。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn stock_screen_revenue_momentum_filters() {
    use chrono::{Datelike, Months};

    let Some(pool) = pool_or_skip("stock_screen_revenue_momentum_filters").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool, &[SEED_STOCK]).await;
    // 新鮮度以查詢當天判斷，營收月份只能由今天往前推；列屬於假代號，結束時清除。
    let today = chrono::Local::now().date_naive();
    let month_code = |back: u32| {
        let month = today - Months::new(back);
        i64::from(month.year()) * 100 + i64::from(month.month())
    };
    // 由新到舊：年增 10、20、5、-3、8 → 連續 3 個月；近 3 月合計 335 對 300 → 11.67%。
    for (back, monthly, yoy) in [
        (1, 110, 10),
        (2, 120, 20),
        (3, 105, 5),
        (4, 97, -3),
        (5, 108, 8),
    ] {
        sqlx::query(
            r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly", "LastYearThisMonth", "ComparedWithLastYearSameMonth")
               VALUES ($1, $1, $2, $3, 100, $4)"#,
        )
        .bind(SYMBOL)
        .bind(month_code(back))
        .bind(Decimal::from(monthly))
        .bind(Decimal::from(yoy))
        .execute(pool)
        .await
        .expect("種入 Revenue");
    }

    let listed = |json: &serde_json::Value| {
        json["stocks"]
            .as_array()
            .expect("stocks 陣列")
            .iter()
            .find(|stock| stock["stock_symbol"] == SYMBOL)
            .cloned()
    };
    let base = "/api/v1/stocks/screen?industry_id=1&limit=50";

    let (status, json) = get(&format!(
        "{base}&min_revenue_yoy_streak=3&min_revenue_yoy_3m_percent=11&sort_by=revenue_yoy_streak&sort_order=desc"
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let stock = listed(&json).expect("符合條件的假代號應出現");
    assert_eq!(stock["revenue_yoy_streak"], 3);
    assert_eq!(stock["revenue_yoy_3m_percent"], 11.67);
    assert_eq!(stock["revenue_yoy_percent"], 10.0);
    assert_eq!(stock["monthly_revenue"], 110.0);
    let (_, json) = get(&format!(
        "{base}&min_monthly_revenue=110&sort_by=monthly_revenue"
    ))
    .await;
    assert!(listed(&json).is_some(), "月營收剛好等於下限應出現");

    for query in [
        "min_revenue_yoy_streak=4",
        "min_revenue_yoy_3m_percent=12",
        "min_monthly_revenue=111",
    ] {
        let (status, json) = get(&format!("{base}&{query}")).await;
        assert_eq!(status, StatusCode::OK, "{query}");
        assert!(listed(&json).is_none(), "{query} 應排除假代號");
    }
    for query in [
        "min_revenue_yoy_streak=0",
        "min_revenue_yoy_streak=13",
        "min_revenue_yoy_3m_percent=-101",
        "min_monthly_revenue=-1",
        "sort_by=revenue_yoy_3months",
    ] {
        let (status, _) = get(&format!("{base}&{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }

    // 拿掉第二新的月份：連續月數停在 1，近 3 月不相連 → null。
    sqlx::query(r#"DELETE FROM "Revenue" WHERE "SecurityCode" = $1 AND "Date" = $2"#)
        .bind(SYMBOL)
        .bind(month_code(2))
        .execute(pool)
        .await
        .expect("刪除一個月份");
    let (_, json) = get(&format!("{base}&min_revenue_yoy_streak=1")).await;
    let stock = listed(&json).expect("仍有 1 個月年增，應出現");
    assert_eq!(stock["revenue_yoy_streak"], 1);
    assert!(stock["revenue_yoy_3m_percent"].is_null(), "{stock}");

    cleanup(pool).await;
}

/// 估值歷史區間：期間以最新有成交的日線往回推，停牌占位列（收盤 0）不算最新一日，
/// 本益比 0（虧損）不算樣本、另計排除天數。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn stock_valuation_band_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("stock_valuation_band_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool, &[SEED_STOCK]).await;

    // 沒有日線：200，各欄位為 null。
    let path = format!("/api/v1/stocks/{SYMBOL}/valuation-band");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert!(json["data_as_of"].is_null(), "{json}");
    assert!(json["price_earning_ratio"].is_null(), "{json}");
    assert_eq!(json["years"], 5);

    seed(
        pool,
        &[r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, year, month, day, "OpeningPrice", "HighestPrice", "LowestPrice", "ClosingPrice", "PriceEarningRatio", "price-to-book_ratio")
             VALUES ('2019-01-10', $1, 2019, 1, 10, 50, 50, 50, 50, 5, 1),
                    ('2020-01-02', $1, 2020, 1, 2, 100, 100, 100, 100, 10, 2),
                    ('2020-03-02', $1, 2020, 3, 2, 120, 120, 120, 120, 0, 2.4),
                    ('2020-05-04', $1, 2020, 5, 4, 150, 150, 150, 150, 15, 3),
                    ('2020-06-30', $1, 2020, 6, 30, 200, 200, 200, 200, 20, 4),
                    ('2020-07-01', $1, 2020, 7, 1, 0, 0, 0, 0, 0, 0)"#],
    )
    .await;

    let (status, json) = get(&format!("{path}?years=1")).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data_as_of"], "2020-06-30");
    assert_eq!(json["from"], "2020-01-02");
    assert_eq!(json["closing_price"], 200.0);
    let pe = &json["price_earning_ratio"];
    assert_eq!(pe["current"], 20.0);
    assert_eq!(pe["percentile"], 100.0);
    assert_eq!(
        (pe["min"].clone(), pe["p25"].clone(), pe["median"].clone()),
        (10.0.into(), 12.5.into(), 15.0.into())
    );
    assert_eq!(
        (pe["samples"].clone(), pe["excluded_days"].clone()),
        (3.into(), 1.into())
    );
    assert_eq!(pe["implied_prices"]["median"], 150.0);
    let pb = &json["price_to_book_ratio"];
    assert_eq!(
        (pb["samples"].clone(), pb["median"].clone()),
        (4.into(), 2.7.into())
    );

    // 兩年的期間納入 2019 的日線。
    let (_, json) = get(&format!("{path}?years=2")).await;
    assert_eq!(json["from"], "2019-01-10");
    assert_eq!(json["price_earning_ratio"]["samples"], 4);

    for query in ["years=0", "years=11"] {
        let (status, _) = get(&format!("{path}?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }
    let (status, _) = get("/api/v1/stocks/79979ZZ/valuation-band").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    cleanup(pool).await;
}

/// 持股總覽：同代號多筆買進合併、已賣出不算；市值、損益、已領股利與市值加權指標，
/// 以及 member_id 篩選與 422。用不存在的會員編號隔離資料，結束時清除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_summary_endpoint_db_semantics() {
    use chrono::Datelike;

    const MEMBER: i64 = 979_790;
    let Some(pool) = pool_or_skip("portfolio_summary_endpoint_db_semantics").await else {
        return;
    };
    let cleanup_portfolio = || async {
        let _ = sqlx::query("DELETE FROM stock_ownership_details WHERE member_id = $1")
            .bind(MEMBER)
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM yield_rank WHERE security_code = $1")
            .bind(SYMBOL)
            .execute(pool)
            .await;
        cleanup(pool).await;
    };
    cleanup_portfolio().await;

    // 沒有持股：200、空清單。
    let path = format!("/api/v1/portfolio/summary?member_id={MEMBER}");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["symbols"], 0);
    assert_eq!(json["holdings"], serde_json::json!([]));
    assert!(json["weighted_dividend_yield"].is_null(), "{json}");

    seed(pool, &[SEED_STOCK]).await;
    // 新鮮度以查詢當天判斷，殖利率日與評分年度只能由今天推算；列屬於假代號，結束時清除。
    let today = chrono::Local::now().date_naive();
    for sql in [
        "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, cumulate_dividends_total, is_sold) VALUES ($1, $2, 1000, -40000, 300, false), ($1, $2, 1000, -50000, 200, false), ($1, $2, 500, -99999, 999, true)",
        "INSERT INTO last_daily_quotes (date, stock_symbol, closing_price) VALUES ('2026-04-30', $2, 50)",
    ] {
        sqlx::query(sql)
            .bind(MEMBER)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("種入持股失敗：{error}\n{sql}"));
    }
    sqlx::query("INSERT INTO yield_rank (date, security_code, yield) VALUES ($1, $2, 4.5)")
        .bind(today)
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("種入 yield_rank");
    sqlx::query("INSERT INTO financial_health (stock_symbol, f_score_year, f_score, f_score_evaluated) VALUES ($1, $2, 6, 9)")
        .bind(SYMBOL)
        .bind(today.year() - 1)
        .execute(pool)
        .await
        .expect("種入 financial_health");

    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["member_id"], MEMBER);
    assert_eq!(json["data_as_of"], "2026-04-30");
    // 兩筆未賣出合併：2,000 股 × 50＝100,000；成本 90,000；已賣出那筆不算。
    assert_eq!(
        (
            json["symbols"].clone(),
            json["market_value"].clone(),
            json["cost"].clone()
        ),
        (1.into(), 100_000.0.into(), 90_000.0.into())
    );
    assert_eq!(json["unrealized_gain"], 10_000.0);
    assert_eq!(json["unrealized_gain_percent"], 11.11);
    assert_eq!(json["dividends_received"], 500.0);
    assert_eq!(json["weighted_dividend_yield"]["value"], 4.5);
    assert_eq!(json["weighted_f_score"]["coverage_percent"], 100.0);
    let holding = &json["holdings"][0];
    assert_eq!(holding["stock_symbol"], SYMBOL);
    assert_eq!(
        (holding["shares"].clone(), holding["weight_percent"].clone()),
        (2_000.into(), 100.0.into())
    );
    assert_eq!(holding["f_score"], 6);
    assert_eq!(json["industries"][0]["industry_id"], 1);

    let (status, _) = get("/api/v1/portfolio/summary?member_id=-1").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    cleanup_portfolio().await;
}

/// 持股績效：扣除買賣金額的時間加權報酬、事後補登的買進記在建立日、已撤銷的賣出不算、
/// 月報酬與最大回撤、同期大盤與超額報酬。用不存在的會員編號與 2099 年的日期隔離資料（全部會員的期間以最新日期往回推，
/// 用未來日期才能確保最新日期是測試資料，不受其他測試寫入的近期市值影響），
/// 大盤列以 `transaction = 979790` 標記，結束時清除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_history_endpoint_db_semantics() {
    const MEMBER: i64 = 979_790;
    let Some(pool) = pool_or_skip("portfolio_history_endpoint_db_semantics").await else {
        return;
    };
    let cleanup_history = || async {
        for sql in [
            "DELETE FROM daily_money_history_member WHERE member_id IN ($1, $1 + 1) OR (member_id = 0 AND date >= '2099-01-01')",
            "DELETE FROM stock_ownership_details WHERE member_id = $1",
            "DELETE FROM sell_history WHERE member_id = $1",
            "DELETE FROM index WHERE category = 'TAIEX' AND transaction = $1",
        ] {
            let _ = sqlx::query(sql).bind(MEMBER).execute(pool).await;
        }
    };
    cleanup_history().await;

    let path = format!("/api/v1/portfolio/history?member_id={MEMBER}");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert!(json["return_percent"].is_null(), "{json}");
    assert_eq!(json["months"], serde_json::json!([]));

    for sql in [
        "INSERT INTO daily_money_history_member (date, member_id, market_value) VALUES ('2099-01-31', $1, 100), ('2099-02-01', $1, 110), ('2099-02-02', $1, 210), ('2099-03-01', $1, 99)",
        // 交易日 01-15（期間前）但 02-01 才補登：記在 02-01。
        "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, date, created_time) VALUES ($1, '79979DA', 1, -10, '2099-01-15', '2099-02-01 10:00+08'), ($1, '79979DA', 1, -100, '2099-02-02', '2099-02-02 10:00+08')",
        "INSERT INTO sell_history (member_id, security_code, date, amount, create_time, is_reverted) VALUES ($1, '79979DA', '2099-03-01', 100, '2099-03-01 10:00+08', false), ($1, '79979DA', '2099-03-01', 999, '2099-03-01 10:00+08', true)",
        "INSERT INTO index (category, date, index, transaction) VALUES ('TAIEX', '2099-01-31', 5000, $1), ('TAIEX', '2099-02-28', 5500, $1), ('TAIEX', '2099-03-01', 5250, $1)",
    ] {
        sqlx::query(sql)
            .bind(MEMBER)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("種入績效資料失敗：{error}\n{sql}"));
    }

    let (status, json) = get(&format!("{path}&months=12")).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(
        (json["from"].clone(), json["data_as_of"].clone()),
        ("2099-01-31".into(), "2099-03-01".into())
    );
    // 02-01：(110 − 10) ÷ 100；02-02：(210 − 100) ÷ 110；03-01：(99 + 100) ÷ 210。
    assert_eq!(json["return_percent"], -5.24);
    assert_eq!(
        (json["buys"].clone(), json["sells"].clone()),
        (110.0.into(), 100.0.into())
    );
    assert_eq!(json["max_drawdown_percent"], -5.24);
    assert_eq!(json["max_drawdown_peak"], "2099-01-31");
    assert_eq!(json["max_drawdown_trough"], "2099-03-01");
    assert_eq!(json["benchmark_return_percent"], 5.0);
    assert_eq!(json["excess_return_percent"], -10.24);
    assert!(json["annualized_return_percent"].is_null());
    let months = json["months"].as_array().expect("months 陣列");
    assert_eq!(months.len(), 2);
    assert_eq!(
        (
            months[0]["month"].clone(),
            months[0]["return_percent"].clone(),
            months[0]["net_contribution"].clone()
        ),
        ("2099-02".into(), 0.0.into(), 110.0.into())
    );
    assert_eq!(months[1]["benchmark_return_percent"], 5.0);

    // 全部會員：另一位會員 02-02 才開始記錄（市值 50），合計當天多出的 50 視為投入。
    // 02-02：(260 − 100 − 50) ÷ 110；03-01：(149 + 100) ÷ 260。
    for sql in [
        "INSERT INTO daily_money_history_member (date, member_id, market_value) VALUES ('2099-02-02', $1 + 1, 50), ('2099-03-01', $1 + 1, 50)",
        "INSERT INTO daily_money_history_member (date, member_id, market_value) VALUES ('2099-01-31', 0, 100), ('2099-02-01', 0, 110), ('2099-02-02', 0, 260), ('2099-03-01', 0, 149)",
    ] {
        sqlx::query(sql)
            .bind(MEMBER)
            .execute(pool)
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "種入合計市值失敗：{error}
{sql}"
                )
            });
    }
    let (status, json) = get("/api/v1/portfolio/history?member_id=0").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["return_percent"], -4.23);
    assert_eq!(json["buys"], 160.0);

    for query in ["member_id=-1", "months=0", "months=121"] {
        let (status, _) = get(&format!("/api/v1/portfolio/history?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }

    cleanup_history().await;
}

/// 股利現金流預估：已公告的配息直接列入、去年同期推一年、全年彙總列不計，
/// 以目前持股股數計算金額與預估殖利率。預估以今天為基準，配息日期只能由今天推算；
/// 列屬於假代號與不存在的會員，結束時清除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_dividend_forecast_endpoint_db_semantics() {
    use chrono::{Datelike, Days};

    const MEMBER: i64 = 979_790;
    let Some(pool) = pool_or_skip("portfolio_dividend_forecast_endpoint_db_semantics").await else {
        return;
    };
    let cleanup_forecast = || async {
        let _ = sqlx::query("DELETE FROM stock_ownership_details WHERE member_id = $1")
            .bind(MEMBER)
            .execute(pool)
            .await;
        cleanup(pool).await;
    };
    cleanup_forecast().await;

    let path = format!("/api/v1/portfolio/dividend-forecast?member_id={MEMBER}");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["total"], 0.0);
    assert_eq!(json["items"], serde_json::json!([]));
    assert_eq!(json["months"].as_array().map(Vec::len), Some(13));

    seed(pool, &[SEED_STOCK]).await;
    let today = chrono::Local::now().date_naive();
    let fmt = |date: chrono::NaiveDate| date.format("%Y-%m-%d").to_string();
    let announced_ex = today + Days::new(20);
    let past_ex = today - Days::new(100);
    sqlx::query(
        "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost) VALUES ($1, $2, 1000, -50000)",
    )
    .bind(MEMBER)
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("種入持股");
    sqlx::query("INSERT INTO last_daily_quotes (date, stock_symbol, closing_price) VALUES ('2026-04-30', $1, 100)")
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("種入報價");
    for (year, year_of_dividend, quarter, cash, ex, payable) in [
        (
            today.year(),
            today.year(),
            "Q4",
            2,
            fmt(announced_ex),
            fmt(announced_ex + Days::new(20)),
        ),
        (
            past_ex.year(),
            past_ex.year(),
            "Q1",
            1,
            fmt(past_ex),
            fmt(past_ex + Days::new(20)),
        ),
        (
            today.year(),
            today.year() - 1,
            "",
            10,
            "-".to_owned(),
            "-".to_owned(),
        ),
    ] {
        sqlx::query(r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2) VALUES ($1, $2, $3, $4, $5, 0, $5, $6, '-', $7, '-')"#)
            .bind(SYMBOL)
            .bind(year)
            .bind(year_of_dividend)
            .bind(quarter)
            .bind(Decimal::from(cash))
            .bind(ex)
            .bind(payable)
            .execute(pool)
            .await
            .expect("種入 dividend");
    }

    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    // 已公告 2 元 × 1,000 股＋去年同期 1 元 × 1,000 股；全年彙總列不計。
    assert_eq!(
        (
            json["announced_total"].clone(),
            json["estimated_total"].clone(),
            json["total"].clone()
        ),
        (2000.0.into(), 1000.0.into(), 3000.0.into())
    );
    assert_eq!(json["forward_yield_percent"], 3.0);
    let items = json["items"].as_array().expect("items 陣列");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["basis"], "announced");
    assert_eq!(items[0]["ex_date"], fmt(announced_ex));
    assert_eq!(items[1]["basis"], "estimated");

    let (_, json) = get(&format!("{path}&months=1")).await;
    assert_eq!(json["estimated_total"], 0.0, "一個月內只有已公告的那筆");

    for query in ["member_id=-1", "months=0", "months=25"] {
        let (status, _) = get(&format!("/api/v1/portfolio/dividend-forecast?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }

    cleanup_forecast().await;
}

/// 填息分析：同日多列只算一次（季別與月別重複、年度彙總列）、同日除權與除息合併、
/// 尚未到的除息日略過；已填息算交易日數，未填息算距離填息價的百分比。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn stock_dividend_fill_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("stock_dividend_fill_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool, &[SEED_STOCK]).await;

    // 沒有日線與股利：統計為 0、data_as_of 為 null。
    let path = format!("/api/v1/stocks/{SYMBOL}/dividend-fill");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json["data_as_of"].is_null(), "{json}");
    assert_eq!(json["events"], serde_json::json!([]));
    assert_eq!(json["summary"]["events"], 0);
    assert!(json["summary"]["average_trading_days_to_fill"].is_null());

    seed(
        pool,
        &[
            r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2")
               VALUES ($1, 2020, 2019, 'Q4', 2.5, 0, 2.5, '2020-03-17', '-'),
                      ($1, 2020, 2019, 'M12', 2.5, 0, 2.5, '2020-03-17', '-'),
                      ($1, 2020, 2019, '', 10, 0, 10, '2020-03-17', '-'),
                      ($1, 2020, 2020, 'Q1', 2.6, 0, 2.6, '2020-06-16', '-'),
                      ($1, 2020, 2020, 'Q2', 0, 0.5, 0.5, '-', '2020-06-16'),
                      ($1, 2020, 2020, 'Q3', 3, 0, 3, '2020-12-15', '-'),
                      ($1, 2021, 2020, 'Q4', 3, 0, 3, '尚未公布', '-')"#,
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, year, month, day, "OpeningPrice", "HighestPrice", "LowestPrice", "ClosingPrice")
               VALUES ('2020-03-16', $1, 2020, 3, 16, 100, 100, 100, 100),
                      ('2020-03-17', $1, 2020, 3, 17, 97.5, 97.5, 97.5, 97.5),
                      ('2020-03-18', $1, 2020, 3, 18, 99, 99, 99, 99),
                      ('2020-03-19', $1, 2020, 3, 19, 100.5, 100.5, 100.5, 100.5),
                      ('2020-06-15', $1, 2020, 6, 15, 110, 110, 110, 110),
                      ('2020-06-16', $1, 2020, 6, 16, 105, 105, 105, 105),
                      ('2020-06-17', $1, 2020, 6, 17, 104, 104, 104, 104),
                      ('2020-06-30', $1, 2020, 6, 30, 106, 106, 106, 106)"#,
        ],
    )
    .await;

    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data_as_of"], "2020-06-30");
    let events = json["events"].as_array().expect("events 陣列");
    let dates: Vec<_> = events
        .iter()
        .map(|e| e["ex_date"].as_str().unwrap())
        .collect();
    assert_eq!(
        dates,
        ["2020-06-16", "2020-03-17"],
        "尚未到的 12-15 與未公布的列都略過"
    );

    let june = &events[0];
    assert_eq!(june["cash_dividend"], 2.6);
    assert_eq!(june["stock_dividend"], 0.5);
    assert_eq!(june["pre_ex_date"], "2020-06-15");
    assert_eq!(june["pre_ex_close"], 110.0);
    assert_eq!(june["filled"], false);
    assert!(june["fill_date"].is_null());
    assert_eq!(june["trading_days_since_ex"], 3);
    assert_eq!(june["latest_close"], 106.0);
    assert_eq!(june["gap_percent"], 3.64);

    let march = &events[1];
    assert_eq!(
        march["cash_dividend"], 2.5,
        "同日的彙總列與月別重複列都不重複計算"
    );
    assert_eq!(march["filled"], true);
    assert_eq!(march["fill_date"], "2020-03-19");
    assert_eq!(march["trading_days_to_fill"], 3);
    assert!(march["gap_percent"].is_null());

    assert_eq!(json["summary"]["events"], 2);
    assert_eq!(json["summary"]["filled"], 1);
    assert_eq!(json["summary"]["average_trading_days_to_fill"], 3.0);
    assert_eq!(json["summary"]["median_trading_days_to_fill"], 3.0);

    let (_, json) = get(&format!("{path}?limit=1")).await;
    assert_eq!(json["events"].as_array().map(Vec::len), Some(1));

    for query in ["limit=0", "limit=41"] {
        let (status, _) = get(&format!("{path}?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }
    let (status, _) = get("/api/v1/stocks/79979ZZ/dividend-fill").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    cleanup(pool).await;
}

/// 同業比較：只看同產業、近 30 天有成交、上市櫃未下市的個股；中位數與名次、依市值排序、
/// limit 與 422／404。兩檔假代號放在不存在的產業，結束時清除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn stock_peers_endpoint_db_semantics() {
    use chrono::Datelike;

    const PEER: &str = "79979DB";
    const INDUSTRY: i32 = 97_979;
    let Some(pool) = pool_or_skip("stock_peers_endpoint_db_semantics").await else {
        return;
    };
    let cleanup_peers = || async {
        for sql in [
            "DELETE FROM last_daily_quotes WHERE stock_symbol = $1",
            "DELETE FROM yield_rank WHERE security_code = $1",
            r#"DELETE FROM "Revenue" WHERE "SecurityCode" = $1"#,
            "DELETE FROM financial_health WHERE stock_symbol = $1",
            "DELETE FROM stocks WHERE stock_symbol = $1",
        ] {
            let _ = sqlx::query(sql).bind(PEER).execute(pool).await;
        }
        let _ = sqlx::query("DELETE FROM yield_rank WHERE security_code = $1")
            .bind(SYMBOL)
            .execute(pool)
            .await;
        cleanup(pool).await;
    };
    cleanup_peers().await;

    let today = chrono::Local::now().date_naive();
    let month = i64::from(today.year()) * 100 + i64::from(today.month());
    for (symbol, issued, close, pe, pb, yoy) in [
        (SYMBOL, 1_000_000_i64, 100, 20, 2, 10),
        (PEER, 5_000_000, 50, 10, 1, -5),
    ] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id, issued_share)
               VALUES ($1, $1, $1, 2, $2, $3)"#,
        )
        .bind(symbol)
        .bind(INDUSTRY)
        .bind(issued)
        .execute(pool)
        .await
        .expect("種入 stocks");
        sqlx::query(
            r#"INSERT INTO last_daily_quotes (date, stock_symbol, closing_price, price_earning_ratio, "price-to-book_ratio")
               VALUES ($1, $2, $3, $4, $5)"#,
        )
        .bind(today)
        .bind(symbol)
        .bind(Decimal::from(close))
        .bind(Decimal::from(pe))
        .bind(Decimal::from(pb))
        .execute(pool)
        .await
        .expect("種入 last_daily_quotes");
        sqlx::query(
            r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "ComparedWithLastYearSameMonth")
               VALUES ($1, $1, $2, $3)"#,
        )
        .bind(symbol)
        .bind(month)
        .bind(Decimal::from(yoy))
        .execute(pool)
        .await
        .expect("種入 Revenue");
    }
    sqlx::query("INSERT INTO yield_rank (date, security_code, yield) VALUES ($1, $2, 4.5)")
        .bind(today)
        .bind(PEER)
        .execute(pool)
        .await
        .expect("種入 yield_rank");
    sqlx::query("INSERT INTO financial_health (stock_symbol, f_score_year, f_score, f_score_evaluated) VALUES ($1, $2, 7, 9)")
        .bind(SYMBOL)
        .bind(today.year() - 1)
        .execute(pool)
        .await
        .expect("種入 financial_health");

    let path = format!("/api/v1/stocks/{SYMBOL}/peers");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["industry_id"], INDUSTRY);
    assert_eq!(json["peer_count"], 2);
    assert_eq!(json["data_as_of"], today.to_string());
    assert_eq!(json["target"]["market_cap"], 100_000_000.0);
    assert_eq!(json["target"]["f_score"], 7);
    // 市值：PEER 2.5 億 > 目標 1 億。
    assert_eq!(json["peers"][0]["stock_symbol"], PEER);
    assert_eq!(json["peers"][0]["dividend_yield_percent"], 4.5);
    let metric = |name: &str| {
        json["metrics"]
            .as_array()
            .expect("metrics 是陣列")
            .iter()
            .find(|metric| metric["metric"] == name)
            .cloned()
            .expect("應有此指標")
    };
    let pe = metric("price_earning_ratio");
    assert_eq!(
        (pe["rank"].clone(), pe["industry_median"].clone()),
        (2.into(), 15.0.into())
    );
    let yoy = metric("revenue_yoy_percent");
    assert_eq!(
        (yoy["rank"].clone(), yoy["target"].clone()),
        (1.into(), 10.0.into())
    );
    let dividend = metric("dividend_yield_percent");
    assert!(dividend["rank"].is_null(), "{dividend}");
    assert_eq!(dividend["samples"], 1);

    let (_, json) = get(&format!("{path}?limit=1")).await;
    assert_eq!(json["peers"].as_array().map(Vec::len), Some(1));
    assert_eq!(json["peer_count"], 2);

    for query in ["limit=0", "limit=101"] {
        let (status, _) = get(&format!("{path}?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }
    let (status, _) = get("/api/v1/stocks/79979ZZ/peers").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    cleanup_peers().await;
}
