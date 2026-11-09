//! 條件選股：神奇公式（Greenblatt）盈餘殖利率、資本報酬率與 `sort_by=magic_formula`。

use serde_json::json;

use super::*;

/// 種入 1988-08-10（查詢日）可用的資料：Yahoo 1988 Q2 資產負債表、1987 Q3～1988 Q2 單季營業利益、查詢日收盤
/// 與成交金額（股本皆 100 → 股數 10）。`(代號, 每季營業利益, 季數, 流動資產, 現金, 流動負債, 短期借款, 長期負債,
/// 設備, 收盤)`：
/// - MA：營業利益 40；資本 (100 − 50) + 150 = 200、淨負債 20、收盤 18 → 企業價值 200。殖利率 20%、報酬率 20%。
/// - MB：營業利益 40；資本 50 + 350 = 400、收盤 10 → 企業價值 100。殖利率 40%、報酬率 10%。
/// - ME：營業利益 8；資本 200、收盤 20 → 殖利率 4%、報酬率 4%。
/// - MC：只有三季營業利益（不足四季），兩者皆為 `null`。
/// - MD：流動負債 0（金融業不分流動與非流動），兩者皆為 `null`。
const STOCKS: [(&str, i64, i32, [i64; 6], i64); 5] = [
    ("79979MA", 10, 4, [120, 20, 60, 10, 30, 150], 18),
    ("79979MB", 10, 4, [100, 0, 50, 0, 0, 350], 10),
    ("79979ME", 2, 4, [100, 0, 50, 0, 0, 150], 20),
    ("79979MC", 10, 3, [100, 0, 50, 0, 0, 150], 20),
    ("79979MD", 10, 4, [100, 0, 0, 0, 0, 150], 20),
];

/// 1987 Q3～1988 Q2（舊到新）。
const QUARTERS: [(i32, &str); 4] = [(1987, "Q3"), (1987, "Q4"), (1988, "Q1"), (1988, "Q2")];

/// `sort_by=magic_formula`／`peg` 是白名單內的排序，兩個方向都能通過驗證，且單獨就能查（全市場排行）；
/// 其他排序單獨使用仍需篩選條件。
#[test]
fn screening_accepts_magic_formula_sort() {
    for order in ["asc", "desc"] {
        let mut params = screening_params();
        params.sort_by = Some("magic_formula".to_owned());
        params.sort_order = Some(order.to_owned());
        let validated = validate_screening_params(&params).expect("參數合法");
        assert!(validated.order_by.contains("magic_formula_rank"), "{order}");
    }
    let mut params = screening_params();
    params.sort_by = Some("peg".to_owned());
    assert!(validate_screening_params(&params).is_ok());
    params.sort_by = Some("eps".to_owned());
    assert_eq!(
        validate_screening_params(&params).err(),
        Some("至少需要一個篩選條件")
    );
}

/// 每檔回傳盈餘殖利率、資本報酬率與名次合計；依名次合計由小到大排序，同名次再依代號，沒有資料的排最後。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_ranks_by_magic_formula() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_ranks_by_magic_formula：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol LIKE '79979M_'"#,
            "DELETE FROM income_statement WHERE stock_symbol LIKE '79979M_'",
            "DELETE FROM balance_sheet WHERE stock_symbol LIKE '79979M_'",
            "DELETE FROM stocks WHERE stock_symbol LIKE '79979M_'",
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    cleanup().await;
    for (symbol, ebit, quarters, [assets, cash, liabilities, short_debt, long_debt, ppe], close) in
        STOCKS
    {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '神奇公式', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
        let skip = QUARTERS.len() - usize::try_from(quarters).expect("季數為正");
        for (year, quarter) in QUARTERS.into_iter().skip(skip) {
            sqlx::query(
                "INSERT INTO income_statement (stock_symbol, fiscal_year, period_type, quarter, source, operating_profit)
                 VALUES ($1, $2, 'single', $3, 'yahoo', $4)",
            )
            .bind(symbol)
            .bind(year)
            .bind(quarter)
            .bind(ebit)
            .execute(pool)
            .await
            .expect("種入損益表");
        }
        sqlx::query(
            "INSERT INTO balance_sheet
                 (stock_symbol, fiscal_year, period_type, quarter, source, current_assets, cash_and_equivalents,
                  current_liabilities, short_term_debt, long_term_liabilities, property_plant_equipment, share_capital)
             VALUES ($1, 1988, 'single', 'Q2', 'yahoo', $2, $3, $4, $5, $6, $7, 100)",
        )
        .bind(symbol)
        .bind(assets)
        .bind(cash)
        .bind(liabilities)
        .bind(short_debt)
        .bind(long_debt)
        .bind(ppe)
        .execute(pool)
        .await
        .expect("種入資產負債表");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "TradeValue")
               VALUES ('1988-08-10', $1, $2, 2000000)"#,
        )
        .bind(symbol)
        .bind(Decimal::from(close))
        .execute(pool)
        .await
        .expect("種入日線");
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 8, 10).expect("固定日期合法");
    let mut params = screening_params();
    params.industry_id = Some(1);
    params.min_avg_trade_value = Some(1);
    params.limit = Some(50);
    params.sort_by = Some("magic_formula".to_owned());
    params.sort_order = Some("asc".to_owned());
    let validated = validate_screening_params(&params).expect("參數合法");
    let response = screen_at(&params, validated, query_date).await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body 可讀");
    cleanup().await;
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON 回應");
    // MA 的分子分母：營業利益 40、投入資本 200、企業價值 18 × 10 + 20 = 200；名次在 3 檔有資料的股票中排。
    let first = &json["stocks"][0];
    assert_eq!(
        (
            &first["stock_symbol"],
            &first["operating_income_ttm"],
            &first["invested_capital"],
            &first["enterprise_value"],
            &first["magic_formula_ranked_count"],
        ),
        (
            &json!("79979MA"),
            &json!(40.0),
            &json!(200.0),
            &json!(200.0),
            &json!(3)
        )
    );
    assert_eq!(
        json["stocks"][3]["magic_formula_ranked_count"],
        serde_json::Value::Null
    );
    let stocks: Vec<_> = json["stocks"]
        .as_array()
        .expect("stocks 陣列")
        .iter()
        .filter(|stock| {
            stock["stock_symbol"]
                .as_str()
                .is_some_and(|symbol| symbol.starts_with("79979M"))
        })
        .map(|stock| {
            (
                stock["stock_symbol"].clone(),
                stock["earnings_yield_percent"].clone(),
                stock["return_on_capital_percent"].clone(),
                stock["magic_formula_rank"].clone(),
            )
        })
        .collect();
    let null = serde_json::Value::Null;
    assert_eq!(
        stocks,
        [
            (json!("79979MA"), json!(20.0), json!(20.0), json!(3)),
            (json!("79979MB"), json!(40.0), json!(10.0), json!(3)),
            (json!("79979ME"), json!(4.0), json!(4.0), json!(6)),
            (json!("79979MC"), null.clone(), null.clone(), null.clone()),
            (json!("79979MD"), null.clone(), null.clone(), null),
        ]
    );
}
