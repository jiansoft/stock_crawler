//! 三大財務報表與財務體質 endpoints 的資料庫語意整合測試。

use super::support::{SEED_STOCK, SYMBOL, cleanup, get, pool_or_skip, seed};
use super::*;

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
