//! 股利預估、持股籌碼、持股填息與 ETF 穿透曝險 endpoints 的資料庫語意整合測試。

use rust_decimal::Decimal;

use super::support::{SEED_STOCK, SYMBOL, cleanup, get, pool_or_skip, seed};
use super::*;

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

/// 持股籌碼日報：日期格式錯誤回 422；查詢日之前沒有任何籌碼資料時回 200、`date` 為 null 與空清單。
/// 持股推算與籌碼欄位由倉儲的整合測試驗證。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_chip_endpoint_db_semantics() {
    if pool_or_skip("portfolio_chip_endpoint_db_semantics")
        .await
        .is_none()
    {
        return;
    }
    let (status, _) = get("/api/v1/portfolio/chip?date=2026-13-01").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, json) = get("/api/v1/portfolio/chip?date=1900-01-01").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert!(json["date"].is_null(), "{json}");
    assert_eq!(json["items"], serde_json::json!([]));
    assert_eq!(json["notable"], 0);
}

/// 持股填息：天數範圍檢查，回應帶回看天數、統計與清單（填息規則由 handler 與 domain 單元測試驗證）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_dividend_fill_endpoint_db_semantics() {
    if pool_or_skip("portfolio_dividend_fill_endpoint_db_semantics")
        .await
        .is_none()
    {
        return;
    }
    for days in [29, 731] {
        let (status, _) = get(&format!("/api/v1/portfolio/dividend-fill?days={days}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "days={days}");
    }
    let (status, json) = get("/api/v1/portfolio/dividend-fill").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["days"], 365);
    assert!(json["items"].is_array(), "{json}");
    assert!(json["summary"]["events"].is_number(), "{json}");
}

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
