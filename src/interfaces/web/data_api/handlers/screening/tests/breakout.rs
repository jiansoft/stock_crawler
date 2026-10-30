//! 條件選股：近期區間突破條件的參數驗證與整合測試。

use super::*;

/// 天數 1～20，單獨即可成立篩選。
#[test]
fn screening_range_breakout_days_boundaries() {
    for (days, ok) in [(1, true), (20, true), (0, false), (21, false)] {
        let mut params = screening_params();
        params.range_breakout_days = Some(days);
        let validated = validate_screening_params(&params);
        assert_eq!(validated.is_ok(), ok, "{days}");
        if let Ok(validated) = validated {
            assert_eq!(validated.range_breakout_days, Some(i64::from(days)));
        }
    }
}

/// 以固定歷史查詢日 1988-04-11 執行：最近 3 個有訊號的交易日是 04-07、04-08、04-11。
/// SB 04-08 突破（列入，附突破日與說明）；SC 04-07 突破但 04-11 跌回（不列入）；SD 03-01 突破、之後沒跌回
/// （3 日不列入、20 日列入）；不設條件時說明欄位為 `null`。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
)]
async fn screen_at_filters_recent_range_breakouts() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 screen_at_filters_recent_range_breakouts：無資料庫連接");
        return;
    }
    let pool = database::get_connection();
    let cleanup = || async {
        for sql in [
            "DELETE FROM daily_technical_signal WHERE stock_symbol LIKE '79979S_'",
            "DELETE FROM stocks WHERE stock_symbol IN ('79979SB', '79979SC', '79979SD')",
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    cleanup().await;
    for symbol in ["79979SB", "79979SC", "79979SD"] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
               VALUES ($1, '突破選股', $1, 2, 1)"#,
        )
        .bind(symbol)
        .execute(pool)
        .await
        .expect("種入股票");
    }
    for (symbol, date, kind) in [
        ("79979SB", "1988-04-08", "breakout"),
        ("79979SC", "1988-04-07", "breakout"),
        ("79979SC", "1988-04-11", "failed"),
        ("79979SD", "1988-03-01", "breakout"),
    ] {
        sqlx::query(
            "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
             VALUES ($1::date, $2, 'range_breakout', $3, $2 || ' ' || $3)",
        )
        .bind(date)
        .bind(symbol)
        .bind(kind)
        .execute(pool)
        .await
        .expect("種入訊號");
    }

    let query_date = NaiveDate::from_ymd_opt(1988, 4, 11).expect("固定日期合法");
    let screen = |days: Option<u8>| async move {
        let mut params = screening_params();
        params.industry_id = Some(1);
        params.range_breakout_days = days;
        params.limit = Some(50);
        let validated = validate_screening_params(&params).expect("參數合法");
        let response = screen_at(&params, validated, query_date).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body 可讀");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON 回應");
        json["stocks"]
            .as_array()
            .expect("stocks 陣列")
            .iter()
            .filter(|stock| {
                stock["stock_symbol"]
                    .as_str()
                    .is_some_and(|symbol| symbol.starts_with("79979S"))
            })
            .map(|stock| {
                (
                    stock["stock_symbol"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    stock["range_breakout_date"].clone(),
                    stock["range_breakout_description"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };

    let three_days = screen(Some(3)).await;
    let twenty_days = screen(Some(20)).await;
    let without = screen(None).await;
    cleanup().await;

    assert_eq!(
        three_days,
        [(
            "79979SB".to_owned(),
            serde_json::json!("1988-04-08"),
            serde_json::json!("79979SB breakout")
        )]
    );
    let symbols: Vec<&str> = twenty_days.iter().map(|row| row.0.as_str()).collect();
    assert_eq!(symbols, ["79979SB", "79979SD"]);
    assert!(
        without.iter().all(|row| row.1.is_null() && row.2.is_null()),
        "{without:?}"
    );
}
