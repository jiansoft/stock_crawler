//! `/market/forward-returns` 的資料庫語意整合測試：種未來報酬、重算大盤彙總再查詢。

use axum::http::StatusCode;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::support::{get, pool_or_skip};
use crate::infra::database::repository::forward_return::refresh_market_forward_returns_between;

/// 測試用假代號：FA 上市、FB 上櫃。
const STOCKS: [(&str, i32); 2] = [("79979FA", 2), ("79979FB", 4)];

/// `(基準日, 代號, 5 日, 20 日)` 報酬（%）：跨 1987、1988 兩個年度；1986-01 只在回看 3 年時算到。
const RETURNS: [(&str, &str, i64, Option<i64>); 5] = [
    ("1986-01-06", "79979FA", 10, None),
    ("1987-12-30", "79979FA", 4, Some(10)),
    ("1987-12-30", "79979FB", -2, None),
    ("1988-01-04", "79979FA", 2, None),
    ("1988-01-04", "79979FB", 6, None),
];

fn window() -> (NaiveDate, NaiveDate) {
    (
        NaiveDate::from_ymd_opt(1980, 1, 1).expect("合法日期"),
        NaiveDate::from_ymd_opt(1999, 12, 31).expect("合法日期"),
    )
}

async fn cleanup(pool: &sqlx::PgPool) {
    let symbols: Vec<&str> = STOCKS.iter().map(|row| row.0).collect();
    for sql in [
        "DELETE FROM daily_forward_return WHERE stock_symbol = ANY($1)",
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(&symbols).execute(pool).await;
    }
    let (from, to) = window();
    refresh_market_forward_returns_between(from, to)
        .await
        .expect("重算大盤彙總");
}

async fn seed(pool: &sqlx::PgPool) {
    for (symbol, market_id) in STOCKS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, $1, $1, $2, 1)"#,
        )
        .bind(symbol)
        .bind(market_id)
        .execute(pool)
        .await
        .expect("種入股票");
    }
    for (date, symbol, r5, r20) in RETURNS {
        sqlx::query(
            "INSERT INTO daily_forward_return (date, stock_symbol, return_5d, return_20d)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(NaiveDate::parse_from_str(date, "%Y-%m-%d").expect("合法日期"))
        .bind(symbol)
        .bind(Decimal::from(r5))
        .bind(r20.map(Decimal::from))
        .execute(pool)
        .await
        .expect("種入報酬");
    }
    let (from, to) = window();
    refresh_market_forward_returns_between(from, to)
        .await
        .expect("重算大盤彙總");
}

/// `by_month` 的 `(月份, 年度數, 5 日平均報酬)`。
fn months(json: &serde_json::Value) -> Vec<(i64, i64, f64)> {
    json["by_month"]
        .as_array()
        .expect("by_month")
        .iter()
        .map(|month| {
            (
                month["month"].as_i64().expect("month"),
                month["years"].as_i64().expect("years"),
                month["benchmark"][0]["avg_return"].as_f64().expect("avg"),
            )
        })
        .collect()
}

/// 等權平均與上漲比例、依年度與月份拆分、市場篩選；參數錯誤回 422，期間沒有資料回 404。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn market_forward_returns_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("market_forward_returns_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool).await;

    let (status, json) = get("/api/v1/market/forward-returns?date=1988-01-10&years=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(
        (
            json["from"].clone(),
            json["to"].clone(),
            json["data_as_of"].clone(),
            json["market"].clone()
        ),
        (
            "1987-01-10".into(),
            "1988-01-10".into(),
            "1988-01-04".into(),
            "all".into()
        )
    );
    // 5 日：(4 − 2 + 2 + 6) / 4 = 2.5、上漲 3/4；20 日只有 FA 10%；60 日沒有樣本。
    let b = &json["benchmark"];
    assert_eq!(
        (b[0]["days"].clone(), b[0]["samples"].clone()),
        (5.into(), 4.into())
    );
    assert_eq!(
        (b[0]["avg_return"].clone(), b[0]["win_rate"].clone()),
        (2.5.into(), 0.75.into())
    );
    assert_eq!(b[1]["avg_return"], 10.0);
    assert_eq!(b[2]["samples"], 0);
    assert!(b[2]["avg_return"].is_null(), "{json}");
    let years: Vec<(i64, f64)> = json["by_year"]
        .as_array()
        .expect("by_year")
        .iter()
        .map(|year| {
            (
                year["year"].as_i64().expect("year"),
                year["benchmark"][0]["avg_return"].as_f64().expect("avg"),
            )
        })
        .collect();
    assert_eq!(years, [(1987, 1.0), (1988, 4.0)]);
    // 依月份：12 月只有 1987-12-30（4、−2），1 月只有 1988-01-04（2、6）。
    assert_eq!(months(&json), [(1, 1, 4.0), (12, 1, 1.0)]);

    // 回看 3 年：1 月合併 1986、1988 兩個年度（10、2、6 → 6）。
    let (status, json) = get("/api/v1/market/forward-returns?date=1988-01-10&years=3").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(months(&json), [(1, 2, 6.0), (12, 1, 1.0)]);

    // 只看上櫃：FB 兩天 −2、6。
    let (status, json) =
        get("/api/v1/market/forward-returns?date=1988-01-10&years=1&market=tpex").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["benchmark"][0]["avg_return"], 2.0);
    assert_eq!(json["benchmark"][0]["win_rate"], 0.5);

    let (status, _) = get("/api/v1/market/forward-returns?date=1985-01-01&years=1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for bad in ["years=0", "years=21", "market=hk", "date=1988-1-4"] {
        let (status, _) = get(&format!("/api/v1/market/forward-returns?{bad}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }

    cleanup(pool).await;
}
