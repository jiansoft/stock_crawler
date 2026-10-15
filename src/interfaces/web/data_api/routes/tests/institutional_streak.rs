//! 法人連續買賣超後報酬統計 endpoint 的資料庫語意整合測試。

use axum::http::StatusCode;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::support::{get, pool_or_skip};

/// `(代號, 市場, 產業)`；產業 24 為 CI 種入的「半導體業」。
const STOCKS: [(&str, i32, i32); 2] = [("79979SA", 2, 24), ("79979SB", 4, 99)];

/// `(日, 代號, 外資, 投信)` 淨額，日期為 1986-06-dd（02～06 五個交易日）。
/// - SA：外資連買 4 天後賣超；投信 03～06 連買。
/// - SB：外資連賣 2 天、03 淨額 0（中斷）、05～06 連買；投信都是 0。
const CHIPS: [(u32, &str, i64, i64); 10] = [
    (2, "79979SA", 100, 0),
    (3, "79979SA", 100, 50),
    (4, "79979SA", 100, 50),
    (5, "79979SA", 100, 50),
    (6, "79979SA", -100, 50),
    (2, "79979SB", -100, 0),
    (3, "79979SB", -100, 0),
    (4, "79979SB", 0, 0),
    (5, "79979SB", 100, 0),
    (6, "79979SB", 100, 0),
];

/// `(日, 代號, 5 日, 20 日)` 報酬（%）。
const RETURNS: [(u32, &str, i64, Option<i64>); 6] = [
    (2, "79979SA", 1, None),
    (3, "79979SA", 4, Some(10)),
    (4, "79979SA", 6, None),
    (3, "79979SB", -4, None),
    (5, "79979SB", 3, None),
    (6, "79979SB", -2, None),
];

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(1986, 6, d).expect("固定日期合法")
}

/// 刪除種入的籌碼、報酬與股票主檔。
async fn cleanup(pool: &sqlx::PgPool) {
    let symbols: Vec<&str> = STOCKS.iter().map(|row| row.0).collect();
    for sql in [
        "DELETE FROM chip_daily WHERE stock_symbol = ANY($1)",
        "DELETE FROM daily_forward_return WHERE stock_symbol = ANY($1)",
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(&symbols).execute(pool).await;
    }
}

async fn seed(pool: &sqlx::PgPool) {
    for (symbol, market_id, industry) in STOCKS {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, $1, $1, $2, $3)"#,
        )
        .bind(symbol)
        .bind(market_id)
        .bind(industry)
        .execute(pool)
        .await
        .expect("種入 stocks");
    }
    for (d, symbol, foreign, trust) in CHIPS {
        sqlx::query(
            r#"INSERT INTO chip_daily ("date", stock_symbol, foreign_net, trust_net, dealer_net)
               VALUES ($1, $2, $3, $4, 0)"#,
        )
        .bind(day(d))
        .bind(symbol)
        .bind(foreign)
        .bind(trust)
        .execute(pool)
        .await
        .expect("種入籌碼");
    }
    crate::infra::database::repository::chip::refresh_streaks_for_tests().await;
    for (d, symbol, r5, r20) in RETURNS {
        sqlx::query(
            "INSERT INTO daily_forward_return (date, stock_symbol, return_5d, return_20d)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(day(d))
        .bind(symbol)
        .bind(Decimal::from(r5))
        .bind(r20.map(Decimal::from))
        .execute(pool)
        .await
        .expect("種入報酬");
    }
}

/// 呼叫 endpoint（固定 06-02～06-08），回傳狀態與 JSON。
async fn query(extra: &str) -> (StatusCode, serde_json::Value) {
    get(&format!(
        "/api/v1/market/institutional-streak/performance?from=1986-06-02&to=1986-06-08&{extra}"
    ))
    .await
}

/// 連續滿 N 天的事件認定、方向／法人／市場／產業篩選、超額報酬、期間開頭的回看、參數驗證與查無資料。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn institutional_streak_performance_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("institutional_streak_performance_endpoint_db_semantics").await
    else {
        return;
    };
    cleanup(pool).await;
    seed(pool).await;

    // 預設外資連買 3 天：只有 SA 06-04；迄日 06-08 回推到最後有籌碼的 06-06。
    let (status, json) = query("").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["to"], "1986-06-06");
    assert_eq!(
        (
            &json["investor"],
            &json["side"],
            &json["streak"],
            &json["market"]
        ),
        (
            &serde_json::json!("foreign"),
            &serde_json::json!("buy"),
            &serde_json::json!(3),
            &serde_json::json!("all")
        )
    );
    assert_eq!(json["adjusted"], true);
    assert_eq!(json["occurrences"], 1);
    assert_eq!(json["horizons"][0]["samples"], 1);
    assert_eq!(json["horizons"][0]["avg_return"], 6.0);
    assert_eq!(json["horizons"][1]["samples"], 0);
    assert!(json["horizons"][1]["avg_return"].is_null());
    assert_eq!(json["benchmark"][0]["samples"], 6);

    // 連買 2 天：SA 06-03（+4）與 SB 06-06（−2）；SB 06-04 淨額 0 中斷了前面的賣超。
    // 06-03 基準 (4 − 4) / 2 = 0、06-06 基準 −2 → 超額 (4 + 0) / 2 = 2、勝過基準 1/2。
    let (status, json) = query("streak=2").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["occurrences"], 2);
    let h5 = &json["horizons"][0];
    assert_eq!(h5["avg_return"], 1.0);
    assert_eq!(h5["median_return"], 1.0);
    assert_eq!(h5["win_rate"], 0.5);
    assert_eq!(h5["avg_excess_return"], 2.0);
    assert_eq!(h5["beat_market_rate"], 0.5);
    assert_eq!(json["horizons"][1]["samples"], 1);
    assert_eq!(json["horizons"][1]["avg_return"], 10.0);

    // 其他條件下的事件數與 5 日平均報酬。
    for (extra, occurrences, avg5) in [
        // 第 1 天就算事件，延續的天數不重複：SA 06-02、SB 06-05。
        ("streak=1", 2, Some(2.0)),
        // 連賣 2 天：SB 06-03。
        ("side=sell&streak=2", 1, Some(-4.0)),
        // 投信連買 3 天：SA 06-05（沒有報酬）。
        ("investor=trust", 1, None),
        ("investor=total", 1, Some(6.0)),
        ("market=tpex&streak=2", 1, Some(-2.0)),
        (
            "industry=%E5%8D%8A%E5%B0%8E%E9%AB%94%E6%A5%AD&streak=2",
            1,
            Some(4.0),
        ),
        ("streak=5", 0, None),
    ] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::OK, "{extra}: {json}");
        assert_eq!(json["occurrences"], occurrences, "{extra}");
        assert_eq!(json["horizons"][0]["avg_return"].as_f64(), avg5, "{extra}");
    }
    let (_, json) = query("market=tpex&streak=2").await;
    assert_eq!(json["benchmark"][0]["samples"], 3, "上櫃基準只算 SB");
    let (_, json) = query("industry=%E5%8D%8A%E5%B0%8E%E9%AB%94%E6%A5%AD").await;
    assert_eq!(json["industry"], "半導體業");

    // 期間從 06-04 開始時，仍往前數到 06-02 的買超，06-04 滿 3 天算事件；從 06-05 起就沒有事件。
    for (from, occurrences) in [("1986-06-04", 1), ("1986-06-05", 0)] {
        let (status, json) = get(&format!(
            "/api/v1/market/institutional-streak/performance?from={from}&to=1986-06-06"
        ))
        .await;
        assert_eq!(status, StatusCode::OK, "{from}: {json}");
        assert_eq!(json["occurrences"], occurrences, "{from}");
    }

    // 依年度拆分：只有 1986 一年，年度統計與整段相同；沒有事件時年度為空清單、整段為 0。
    let (status, whole) = query("streak=3").await;
    assert_eq!(status, StatusCode::OK, "{whole}");
    assert!(whole["years"].is_null() && whole["group_by"].is_null());
    let (status, json) = query("streak=3&group_by=year").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["group_by"], "year");
    assert_eq!(json["occurrences"], whole["occurrences"]);
    assert_eq!(json["horizons"], whole["horizons"]);
    assert_eq!(json["years"].as_array().map(Vec::len), Some(1), "{json}");
    assert_eq!(json["years"][0]["year"], 1986);
    assert_eq!(json["years"][0]["occurrences"], whole["occurrences"]);
    assert_eq!(json["years"][0]["horizons"], whole["horizons"]);
    assert_eq!(json["benchmark_by_year"][0]["year"], 1986);
    assert_eq!(
        json["benchmark_by_year"][0]["benchmark"],
        whole["benchmark"]
    );
    let (status, json) = query("streak=60&group_by=year").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["occurrences"], 0);
    assert_eq!(json["years"].as_array().map(Vec::len), Some(0), "{json}");

    for (extra, status) in [
        ("group_by=week", StatusCode::UNPROCESSABLE_ENTITY),
        ("streak=0", StatusCode::UNPROCESSABLE_ENTITY),
        ("streak=61", StatusCode::UNPROCESSABLE_ENTITY),
        ("side=hold", StatusCode::UNPROCESSABLE_ENTITY),
        ("investor=retail", StatusCode::UNPROCESSABLE_ENTITY),
        ("market=otc", StatusCode::UNPROCESSABLE_ENTITY),
        (
            "industry=%E4%B8%8D%E5%AD%98%E5%9C%A8%E7%94%A2%E6%A5%AD",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let (actual, json) = query(extra).await;
        assert_eq!(actual, status, "{extra}: {json}");
        assert!(json["error"].is_string(), "{extra}: {json}");
    }
    for (path, status) in [
        (
            "from=1986-06-07&to=1986-06-06",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("from=1986-13-01", StatusCode::UNPROCESSABLE_ENTITY),
        ("to=1900-01-01", StatusCode::NOT_FOUND),
    ] {
        let (actual, json) = get(&format!(
            "/api/v1/market/institutional-streak/performance?{path}"
        ))
        .await;
        assert_eq!(actual, status, "{path}: {json}");
    }

    cleanup(pool).await;
}
