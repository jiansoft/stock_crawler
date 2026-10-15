//! 相對強度排行 endpoint 的資料庫語意整合測試。

use axum::http::StatusCode;
use chrono::{Datelike, NaiveDate, Weekday};
use rust_decimal::Decimal;

use super::support::{get, pool_or_skip};

/// `(代號, 市場, 產業)`；產業 24 為 CI 種入的「半導體業」，9001 為 ETF（不計分）；
/// 7997Z9 是特別股代號（4 碼數字後接英文字母，不計分）。
const STOCKS: [(&str, i32, i32); 5] = [
    ("79979RA", 2, 24),
    ("79979RB", 4, 24),
    ("79979RC", 2, 24),
    ("79979RE", 2, 9001),
    ("7997Z9", 2, 17),
];

/// 1992 年起的前 `count` 個平日，由舊到新。
fn weekdays(count: usize) -> Vec<NaiveDate> {
    NaiveDate::from_ymd_opt(1992, 1, 6)
        .expect("固定日期合法")
        .iter_days()
        .filter(|day| !matches!(day.weekday(), Weekday::Sat | Weekday::Sun))
        .take(count)
        .collect()
}

async fn cleanup(pool: &sqlx::PgPool) {
    let symbols: Vec<&str> = STOCKS.iter().map(|row| row.0).collect();
    for sql in [
        "DELETE FROM daily_relative_strength WHERE stock_symbol = ANY($1)",
        "DELETE FROM daily_technical_signal WHERE stock_symbol = ANY($1)",
        "DELETE FROM daily_forward_return WHERE stock_symbol = ANY($1)",
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(&symbols).execute(pool).await;
    }
}

/// RA 每個平日都有日線（構成交易日曆），RB 只有資料日有收盤；RC 沒有 240 日報酬，不計分；
/// RE 是 ETF、Z9 是特別股，報酬最高但都不列入母體。
/// 資料日之後再加一個交易日但不回填報酬，用來驗證退回前一個交易日。
async fn seed(pool: &sqlx::PgPool, dates: &[NaiveDate]) {
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
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice")
           SELECT d, '79979RA', 50 FROM UNNEST($1::date[]) AS d"#,
    )
    .bind(dates)
    .execute(pool)
    .await
    .expect("種入 RA 日線");
    let end = dates[dates.len() - 2];
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice") VALUES ($1, '79979RB', 12.5)"#,
    )
    .bind(end)
    .execute(pool)
    .await
    .expect("種入 RB 日線");

    // 資料日往前第 N 個交易日的 N 日報酬。
    let base = |horizon: usize| dates[dates.len() - 2 - horizon];
    for (symbol, horizon, value) in [
        ("79979RA", 20, 1),
        ("79979RA", 60, 10),
        ("79979RA", 120, 20),
        ("79979RA", 240, 30),
        ("79979RB", 60, -5),
        ("79979RB", 120, -10),
        ("79979RB", 240, 0),
        ("79979RC", 60, 50),
        ("79979RC", 120, 50),
        ("79979RE", 60, 90),
        ("79979RE", 120, 90),
        ("79979RE", 240, 90),
        ("7997Z9", 60, 80),
        ("7997Z9", 120, 80),
        ("7997Z9", 240, 80),
    ] {
        sqlx::query(
            "INSERT INTO daily_forward_return AS t (date, stock_symbol, return_20d, return_60d, return_120d, return_240d)
             VALUES ($1, $2,
                     CASE WHEN $4 = 20 THEN $3 END, CASE WHEN $4 = 60 THEN $3 END,
                     CASE WHEN $4 = 120 THEN $3 END, CASE WHEN $4 = 240 THEN $3 END)
             ON CONFLICT (date, stock_symbol) DO UPDATE
             SET return_20d = COALESCE(EXCLUDED.return_20d, t.return_20d),
                 return_60d = COALESCE(EXCLUDED.return_60d, t.return_60d),
                 return_120d = COALESCE(EXCLUDED.return_120d, t.return_120d),
                 return_240d = COALESCE(EXCLUDED.return_240d, t.return_240d)",
        )
        .bind(base(horizon))
        .bind(symbol)
        .bind(Decimal::from(value))
        .bind(i32::try_from(horizon).expect("天期很小"))
        .execute(pool)
        .await
        .expect("種入報酬");
    }
}

/// 以全體計分；篩選只影響輸出；資料日還沒回填時退回前一個交易日；參數錯誤 422、查無 404。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn relative_strength_ranks_market_universe() {
    let Some(pool) = pool_or_skip("relative_strength_ranks_market_universe").await else {
        return;
    };
    // 241 個交易日的資料日，再加一個沒有回填報酬的交易日。
    let dates = weekdays(242);
    let end = dates[dates.len() - 2];
    let latest = dates[dates.len() - 1];
    cleanup(pool).await;
    seed(pool, &dates).await;

    let (status, json) = get(&format!("/api/v1/market/relative-strength?date={latest}")).await;
    let (_, rb_only) = get(&format!(
        "/api/v1/market/relative-strength?date={end}&symbol=79979RB"
    ))
    .await;
    let (_, tpex) = get(&format!(
        "/api/v1/market/relative-strength?date={end}&market=tpex&order=asc"
    ))
    .await;
    let (_, semi) = get(&format!(
        "/api/v1/market/relative-strength?date={end}&industry=%E5%8D%8A%E5%B0%8E%E9%AB%94%E6%A5%AD&limit=1"
    ))
    .await;
    let mut errors = Vec::new();
    for path in [
        "/api/v1/market/relative-strength?order=up",
        "/api/v1/market/relative-strength?limit=0",
        "/api/v1/market/relative-strength?market=xx",
        "/api/v1/market/relative-strength?date=1992-13-01",
        "/api/v1/market/relative-strength?symbol=79979ZZ",
        "/api/v1/market/relative-strength?date=1980-01-01",
    ] {
        errors.push(get(path).await.0);
    }
    cleanup(pool).await;

    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(
        json["data_as_of"],
        end.to_string(),
        "最新交易日沒有報酬時退回前一天"
    );
    assert_eq!(json["universe"], 2);
    assert_eq!(json["items"][0]["stock_symbol"], "79979RA");
    assert_eq!(json["items"][0]["rank"], 1);
    assert_eq!(json["items"][0]["rs_score"], 99);
    // 0.4×10 + 0.3×20 + 0.3×30 = 19。
    assert_eq!(json["items"][0]["weighted_return"], 19.0);
    assert_eq!(json["items"][0]["return_20d"], 1.0);
    assert_eq!(json["items"][0]["return_240d"], 30.0);
    assert_eq!(json["items"][0]["closing_price"], 50.0);
    assert_eq!(json["items"][0]["industry"], "半導體業");
    assert_eq!(json["items"][1]["stock_symbol"], "79979RB");
    assert_eq!(json["items"][1]["rs_score"], 1);
    assert!(json["items"][1]["return_20d"].is_null());
    assert_eq!(json["items"][1]["closing_price"], 12.5);
    assert_eq!(json["items"][1]["market"], "tpex");

    assert_eq!(rb_only["matched"], 1);
    assert_eq!(rb_only["items"][0]["rank"], 2);
    assert_eq!(tpex["order"], "asc");
    assert_eq!(tpex["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(semi["matched"], 2);
    assert_eq!(semi["items"].as_array().map(Vec::len), Some(1));

    assert_eq!(
        errors,
        [
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::NOT_FOUND,
            StatusCode::NOT_FOUND,
        ]
    );
}

/// 個股歷史與排行同一算法：資料日的分數、名次、母體一致；沒有回填報酬的交易日略過；
/// 未計分的股票回空清單；不存在 404、days 超界 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn relative_strength_history_matches_ranking() {
    let Some(pool) = pool_or_skip("relative_strength_history_matches_ranking").await else {
        return;
    };
    let dates = weekdays(242);
    let end = dates[dates.len() - 2];
    let latest = dates[dates.len() - 1];
    cleanup(pool).await;
    seed(pool, &dates).await;
    // 前一個交易日只有 RB 有分數（60 分）：資料日 RB 掉到 1 分，產生跌破 50 分的訊號；RA 前一天
    // 沒有分數，不產生訊號。
    sqlx::query(
        "INSERT INTO daily_relative_strength (date, stock_symbol, weighted_return, rs_score, rank, universe)
         VALUES ($1, '79979RB', 0, 60, 1, 1)",
    )
    .bind(dates[dates.len() - 3])
    .execute(pool)
    .await
    .expect("種入前一天分數");
    // 個股歷史讀每日分數：資料日寫入 2 檔（重跑一次確認穿越訊號整天替換）；最新一天報酬還沒回填，不寫入。
    for (day, expected) in [(end, 2), (end, 2), (latest, 0)] {
        let written = crate::app::calculation::relative_strength::calculate_relative_strength(day)
            .await
            .expect("寫入每日相對強度");
        assert_eq!(written, expected, "{day}");
    }
    let crossings: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT stock_symbol, indicator, kind, description FROM daily_technical_signal
         WHERE date = $1 AND stock_symbol LIKE '79979R_'",
    )
    .bind(end)
    .fetch_all(pool)
    .await
    .expect("讀回穿越訊號");

    let (status, ra) = get(&format!(
        "/api/v1/stocks/79979RA/relative-strength?date={latest}&days=2"
    ))
    .await;
    let (_, rb) = get(&format!(
        "/api/v1/stocks/79979RB/relative-strength?date={end}"
    ))
    .await;
    let (rc_status, rc) = get(&format!(
        "/api/v1/stocks/79979RC/relative-strength?date={end}"
    ))
    .await;
    let mut errors = Vec::new();
    for path in [
        "/api/v1/stocks/79979ZZ/relative-strength",
        "/api/v1/stocks/79979RA/relative-strength?days=0",
        "/api/v1/stocks/79979RA/relative-strength?days=121",
        "/api/v1/stocks/79979RA/relative-strength?date=1992-02-30",
    ] {
        errors.push(get(path).await.0);
    }
    cleanup(pool).await;

    assert_eq!(status, StatusCode::OK, "{ra}");
    assert_eq!(ra["days"], 2);
    assert_eq!(
        ra["data_as_of"],
        end.to_string(),
        "最新交易日沒有報酬時略過"
    );
    assert_eq!(ra["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(ra["items"][0]["rs_score"], 99);
    assert_eq!(ra["items"][0]["rank"], 1);
    assert_eq!(ra["items"][0]["universe"], 2);
    assert_eq!(ra["items"][0]["weighted_return"], 19.0);
    assert_eq!(ra["latest_score"], 99);
    assert_eq!(ra["score_change"], 0);
    assert_eq!(rb["items"][0]["rs_score"], 1);
    assert_eq!(rb["items"][0]["rank"], 2);
    assert_eq!(rb["low_date"], end.to_string());
    assert_eq!(
        crossings,
        [(
            "79979RB".to_owned(),
            "rs50".to_owned(),
            "cross_below".to_owned(),
            "相對強度 60 → 1 分，跌破 50 分".to_owned()
        )]
    );

    assert_eq!(rc_status, StatusCode::OK);
    assert!(rc["data_as_of"].is_null());
    assert_eq!(rc["items"].as_array().map(Vec::len), Some(0));

    assert_eq!(
        errors,
        [
            StatusCode::NOT_FOUND,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::UNPROCESSABLE_ENTITY,
        ]
    );
}

/// 產業相對強度：成分股分數取自全體計分；檔數門檻、市場篩選；前期沒有資料時比較欄位為 null；
/// 參數錯誤 422、查無 404。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn industry_strength_aggregates_scores() {
    let Some(pool) = pool_or_skip("industry_strength_aggregates_scores").await else {
        return;
    };
    let dates = weekdays(242);
    let end = dates[dates.len() - 2];
    cleanup(pool).await;
    seed(pool, &dates).await;

    let (status, json) = get(&format!(
        "/api/v1/market/industry-strength?date={end}&min_stocks=1"
    ))
    .await;
    let (_, otc) = get(&format!(
        "/api/v1/market/industry-strength?date={end}&market=tpex&min_stocks=1&order=asc"
    ))
    .await;
    let (_, strict) = get(&format!(
        "/api/v1/market/industry-strength?date={end}&min_stocks=3"
    ))
    .await;
    let mut errors = Vec::new();
    for path in [
        "/api/v1/market/industry-strength?order=up",
        "/api/v1/market/industry-strength?min_stocks=0",
        "/api/v1/market/industry-strength?market=xx",
        "/api/v1/market/industry-strength?date=1980-01-01",
    ] {
        errors.push(get(path).await.0);
    }
    cleanup(pool).await;

    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data_as_of"], end.to_string());
    assert!(json["prior_as_of"].is_null(), "{json}");
    assert_eq!(json["universe"], 2);
    let semi = &json["items"][0];
    assert_eq!(semi["industry"], "半導體業");
    assert_eq!(semi["stocks"], 2);
    // RA 99、RB 1 → 中位數 50。
    assert_eq!(semi["median_rs_score"], 50.0);
    assert_eq!(semi["strong_ratio"], 0.5);
    assert_eq!(semi["weak_ratio"], 0.5);
    assert_eq!(semi["leaders"][0]["stock_symbol"], "79979RA");
    assert!(semi["median_change"].is_null());

    assert_eq!(otc["order"], "asc");
    assert_eq!(otc["items"][0]["stocks"], 1);
    assert_eq!(otc["items"][0]["median_rs_score"], 1.0);
    assert_eq!(strict["items"].as_array().map(Vec::len), Some(0));

    assert_eq!(
        errors,
        [
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::UNPROCESSABLE_ENTITY,
            StatusCode::NOT_FOUND,
        ]
    );
}
