//! 股利預估、持股籌碼與持股填息 endpoints 的資料庫語意整合測試。

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
        for sql in [
            "DELETE FROM dividend_record_detail_more WHERE stock_ownership_details_serial IN (SELECT serial FROM stock_ownership_details WHERE member_id = $1)",
            "DELETE FROM stock_ownership_details WHERE member_id = $1",
        ] {
            let _ = sqlx::query(sql).bind(MEMBER).execute(pool).await;
        }
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
    let recorded_ex = today - Days::new(5);
    // 除息前持有 600 股（有已領明細）、除息後再買 400 股；目前合計 1,000 股。
    let mut lots = Vec::new();
    for shares in [600_i64, 400] {
        let serial: i64 = sqlx::query_scalar(
            "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost) VALUES ($1, $2, $3, -50000) RETURNING serial",
        )
        .bind(MEMBER)
        .bind(SYMBOL)
        .bind(shares)
        .fetch_one(pool)
        .await
        .expect("種入持股");
        lots.push(serial);
    }
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
        (
            today.year(),
            today.year(),
            "Q3",
            1,
            fmt(recorded_ex),
            fmt(recorded_ex + Days::new(15)),
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
    sqlx::query(
        "INSERT INTO dividend_record_detail_more (stock_ownership_details_serial, dividend_record_detail_serial, dividend_serial, cash, total) SELECT $1, 0, serial, 600, 600 FROM dividend WHERE security_code = $2 AND quarter = 'Q3'",
    )
    .bind(lots[0])
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("種入已領明細");

    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    // 已公告 2 元 × 1,000 股＋已除息 1 元依除息日 600 股（已領明細）＋去年同期那期依最近一期
    // 2 元 × 1,000 股（季配）；全年彙總列不計。
    assert_eq!(
        (
            json["announced_total"].clone(),
            json["estimated_total"].clone(),
            json["total"].clone()
        ),
        (2600.0.into(), 2000.0.into(), 4600.0.into())
    );
    assert_eq!(json["forward_yield_percent"], 4.6);
    let items = json["items"].as_array().expect("items 陣列");
    assert_eq!(items.len(), 3, "{items:?}");
    assert_eq!(
        (
            &items[0]["shares"],
            &items[0]["entitled"],
            &items[0]["ex_date"]
        ),
        (&600.into(), &true.into(), &fmt(recorded_ex).into())
    );
    assert_eq!(items[1]["basis"], "announced");
    assert_eq!(items[1]["entitled"], false);
    assert_eq!(items[1]["ex_date"], fmt(announced_ex));
    assert_eq!(items[2]["basis"], "latest_amount");

    let (_, json) = get(&format!("{path}&months=1")).await;
    assert_eq!(json["estimated_total"], 0.0, "一個月內只有已公告的那兩筆");

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

/// 持股填息：天數與會員範圍檢查，回應帶會員、回看天數、統計與清單（填息規則由 handler 與 domain 單元測試驗證）。
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
    let (status, _) = get("/api/v1/portfolio/dividend-fill?member_id=-1").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "member_id=-1");
    let (status, json) = get("/api/v1/portfolio/dividend-fill?member_id=979792").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["member_id"], 979_792);
    assert_eq!(
        json["items"],
        serde_json::json!([]),
        "沒有持股的會員沒有事件"
    );
    let (status, json) = get("/api/v1/portfolio/dividend-fill").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["member_id"], 0);
    assert_eq!(json["days"], 365);
    assert!(json["items"].is_array(), "{json}");
    assert!(json["summary"]["events"].is_number(), "{json}");
}
