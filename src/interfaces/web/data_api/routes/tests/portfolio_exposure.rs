//! ETF 穿透曝險、近期股利與股利預估準確度 endpoints 的資料庫語意整合測試。

use rust_decimal::Decimal;

use super::support::{SEED_STOCK, SYMBOL, cleanup, get, pool_or_skip, seed};
use super::*;

/// 持股穿透：持有的假 ETF 依前十大持股拆到成分股，與直接持有的假股票合併；
/// 沒有收盤價的部位列入 unpriced；member_id 篩選與 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_etf_exposure_endpoint_db_semantics() {
    const MEMBER: i64 = 979_791;
    const ETF: &str = "79979EX";
    const UNPRICED: &str = "79979UP";
    let Some(pool) = pool_or_skip("portfolio_etf_exposure_endpoint_db_semantics").await else {
        return;
    };
    let cleanup_exposure = || async {
        for sql in [
            "DELETE FROM stock_ownership_details WHERE member_id = $1",
            "DELETE FROM etf_holding WHERE stock_symbol = $2",
            "DELETE FROM etf_snapshot WHERE stock_symbol = $2",
            "DELETE FROM last_daily_quotes WHERE stock_symbol = $2",
        ] {
            let _ = sqlx::query(sql).bind(MEMBER).bind(ETF).execute(pool).await;
        }
        cleanup(pool).await;
    };
    cleanup_exposure().await;

    let path = format!("/api/v1/portfolio/etf-exposure?member_id={MEMBER}");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["total_value"], 0.0);
    assert_eq!(json["items"], serde_json::json!([]));

    seed(pool, &[SEED_STOCK]).await;
    for sql in [
        "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, is_sold) VALUES ($1, $2, 1000, -1, false), ($1, $3, 2000, -1, false), ($1, $4, 10, -1, false), ($1, $2, 500, -1, true)",
        "INSERT INTO last_daily_quotes (date, stock_symbol, closing_price) VALUES ('2026-04-30', $2, 50), ('2026-04-30', $3, 100)",
        "INSERT INTO etf_snapshot (stock_symbol, name, market_id) VALUES ($3, '端點測試 ETF', 2)",
        "INSERT INTO etf_holding (stock_symbol, rank, as_of, holding_symbol, holding_name, weight) VALUES ($3, 1, '2026-03-31', $2, '端點測試股', 25), ($3, 2, '2026-03-31', '', '現金', 5)",
    ] {
        sqlx::query(sql)
            .bind(MEMBER)
            .bind(SYMBOL)
            .bind(ETF)
            .bind(UNPRICED)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("種入持股失敗：{error}\n{sql}"));
    }

    let (status, json) = get(&format!("{path}&limit=1")).await;
    cleanup_exposure().await;
    assert_eq!(status, StatusCode::OK, "{json}");
    // 直接 1000 × 50 = 50,000；ETF 2000 × 100 = 200,000，25% 拆到同一檔、5% 為現金。
    assert_eq!(json["total_value"], 250_000.0);
    assert_eq!(json["etf_value"], 200_000.0);
    assert_eq!(json["uncovered_value"], 140_000.0);
    assert_eq!(json["total_items"], 2);
    assert_eq!(json["unpriced"], serde_json::json!([UNPRICED]));
    assert_eq!(json["etfs"][0]["name"], "端點測試 ETF");
    assert_eq!(json["etfs"][0]["holdings_as_of"], "2026-03-31");
    let items = json["items"].as_array().expect("items 是陣列");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["symbol"], SYMBOL);
    assert_eq!(items[0]["name"], "端點測試股");
    assert_eq!(items[0]["direct_value"], 50_000.0);
    assert_eq!(items[0]["etf_value"], 50_000.0);
    assert_eq!(items[0]["weight"], 40.0);
    assert_eq!(items[0]["sources"][0]["etf_symbol"], ETF);

    for bad in [
        "/api/v1/portfolio/etf-exposure?member_id=-1",
        "/api/v1/portfolio/etf-exposure?limit=0",
        "/api/v1/portfolio/etf-exposure?limit=201",
    ] {
        let (status, json) = get(bad).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad} {json}");
    }
}

/// 持股近期除權息：只列期間內已公告的持股除權息，依持股股數算入帳與配股；沒有持股回空清單；
/// 參數超出範圍回 422。配息日期只能由今天推算；列屬於假代號與不存在的會員，結束時清除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_upcoming_dividends_endpoint_db_semantics() {
    use chrono::{Datelike, Days};

    const MEMBER: i64 = 979_791;
    let Some(pool) = pool_or_skip("portfolio_upcoming_dividends_endpoint_db_semantics").await
    else {
        return;
    };
    let cleanup_upcoming = || async {
        let _ = sqlx::query("DELETE FROM stock_ownership_details WHERE member_id = $1")
            .bind(MEMBER)
            .execute(pool)
            .await;
        cleanup(pool).await;
    };
    cleanup_upcoming().await;

    let path = format!("/api/v1/portfolio/upcoming-dividends?member_id={MEMBER}&days=10");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["items"], serde_json::json!([]), "沒有持股");
    assert_eq!(json["total_cash_amount"], 0.0);

    seed(pool, &[SEED_STOCK]).await;
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
    let today = chrono::Local::now().date_naive();
    let soon = today + Days::new(3);
    let later = today + Days::new(30);
    for (ex, cash, stock) in [(soon, 2, 1), (later, 3, 0)] {
        sqlx::query(r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2) VALUES ($1, $2, $2, $3, $4, $5, $4 + $5, $6, $6, '-', '-')"#)
            .bind(SYMBOL)
            .bind(ex.year())
            .bind(if ex == soon { "Q3" } else { "Q4" })
            .bind(Decimal::from(cash))
            .bind(Decimal::from(stock))
            .bind(ex.format("%Y-%m-%d").to_string())
            .execute(pool)
            .await
            .expect("種入 dividend");
    }

    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let items = json["items"].as_array().expect("items 陣列");
    assert_eq!(items.len(), 1, "30 天後那筆在 10 天以外：{json}");
    assert_eq!(items[0]["ex_date"], soon.format("%Y-%m-%d").to_string());
    assert_eq!(items[0]["shares"], 1000);
    assert_eq!(items[0]["cash_amount"], 2000.0);
    assert_eq!(items[0]["stock_shares"], 100, "1000 股 × 1 元 ÷ 面額 10 元");
    assert_eq!(json["total_cash_amount"], 2000.0);
    assert_eq!(json["from"], today.format("%Y-%m-%d").to_string());

    for query in ["member_id=-1", "days=0", "days=61"] {
        let (status, _) = get(&format!("/api/v1/portfolio/upcoming-dividends?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }

    cleanup_upcoming().await;
}

/// 股利預估誤差：指定快照日依（代號, 發放月份）比對已領明細；同月入帳為 matched、
/// 月份已過沒入帳為 missed、發放日未到為 pending。快照日固定用歷史日期，列屬於假代號與
/// 不存在的會員，結束時清除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_dividend_forecast_accuracy_endpoint_db_semantics() {
    use chrono::Days;

    const MEMBER: i64 = 979_791;
    const SNAPSHOT: &str = "2026-04-30";
    let Some(pool) =
        pool_or_skip("portfolio_dividend_forecast_accuracy_endpoint_db_semantics").await
    else {
        return;
    };
    let cleanup_accuracy = || async {
        for sql in [
            "DELETE FROM dividend_forecast_snapshot WHERE member_id = $1",
            "DELETE FROM dividend_record_detail_more WHERE stock_ownership_details_serial IN (SELECT serial FROM stock_ownership_details WHERE member_id = $1)",
            "DELETE FROM stock_ownership_details WHERE member_id = $1",
        ] {
            let _ = sqlx::query(sql).bind(MEMBER).execute(pool).await;
        }
        cleanup(pool).await;
    };
    cleanup_accuracy().await;

    let path = format!("/api/v1/portfolio/dividend-forecast/accuracy?member_id={MEMBER}");
    let (status, json) = get(&format!("{path}&snapshot_date=1900-01-01")).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["snapshot_date"], "1900-01-01");
    assert_eq!(json["items"], serde_json::json!([]));

    seed(pool, &[SEED_STOCK]).await;
    let lot: i64 = sqlx::query_scalar(
        "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost) VALUES ($1, $2, 1000, -50000) RETURNING serial",
    )
    .bind(MEMBER)
    .bind(SYMBOL)
    .fetch_one(pool)
    .await
    .expect("種入持股");
    sqlx::query(r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2) VALUES ($1, 2026, 2026, 'Q1', 0.6, 0, 0.6, '2026-05-02', '-', '2026-05-22', '-')"#)
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("種入 dividend");
    sqlx::query(
        "INSERT INTO dividend_record_detail_more (stock_ownership_details_serial, dividend_record_detail_serial, dividend_serial, cash, total) SELECT $1, 0, serial, 600, 600 FROM dividend WHERE security_code = $2 AND quarter = 'Q1'",
    )
    .bind(lot)
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("種入已領明細");
    let pending_pay = chrono::Local::now().date_naive() + Days::new(30);
    for (ex, pay, amount, basis) in [
        ("2026-05-01", "2026-05-20".to_owned(), 500, "announced"),
        ("2026-06-01", "2026-06-15".to_owned(), 300, "estimated"),
        ("2026-12-20", pending_pay.to_string(), 700, "latest_amount"),
    ] {
        sqlx::query(
            "INSERT INTO dividend_forecast_snapshot (snapshot_date, member_id, stock_symbol, ex_date, pay_date, shares, cash_per_share, amount, basis) VALUES ($1::date, $2, $3, $4::date, $5::date, 1000, 0.5, $6, $7)",
        )
        .bind(SNAPSHOT)
        .bind(MEMBER)
        .bind(SYMBOL)
        .bind(ex)
        .bind(pay)
        .bind(Decimal::from(amount))
        .bind(basis)
        .execute(pool)
        .await
        .expect("種入快照");
    }

    let (status, json) = get(&format!("{path}&snapshot_date={SNAPSHOT}")).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let items = json["items"].as_array().expect("items 陣列");
    let rows: Vec<(String, String, String)> = items
        .iter()
        .map(|item| {
            (
                item["month"].as_str().unwrap_or_default().to_owned(),
                item["basis"].as_str().unwrap_or_default().to_owned(),
                item["status"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        rows[..2],
        [
            (
                "2026-05".to_owned(),
                "announced".to_owned(),
                "matched".to_owned()
            ),
            (
                "2026-06".to_owned(),
                "estimated".to_owned(),
                "missed".to_owned()
            ),
        ],
        "{json}"
    );
    assert_eq!(rows[2].2, "pending");
    assert_eq!(items[0]["name"], "端點測試股");
    assert_eq!(
        (
            &items[0]["actual"],
            &items[0]["difference"],
            &items[0]["error_percent"]
        ),
        (&600.0.into(), &100.0.into(), &16.67.into())
    );
    assert_eq!(items[2]["actual"], serde_json::Value::Null);
    assert_eq!(
        (
            &json["settled_forecast"],
            &json["settled_actual"],
            &json["pending_count"]
        ),
        (&800.0.into(), &600.0.into(), &1.into())
    );
    assert_eq!(json["by_basis"][0]["basis"], "announced");
    assert_eq!(json["by_basis"][0]["median_error_percent"], 16.67);

    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "未指定快照日時取最近一次：{json}");
    assert!(json["snapshot_date"].is_string(), "{json}");

    for query in ["member_id=-1", "snapshot_date=2026-13-01"] {
        let (status, _) = get(&format!(
            "/api/v1/portfolio/dividend-forecast/accuracy?{query}"
        ))
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }

    cleanup_accuracy().await;
}
