//! 估值歷史區間、填息分析與同業比較 endpoints 的資料庫語意整合測試。

use rust_decimal::Decimal;

use super::support::{SEED_STOCK, SYMBOL, cleanup, get, pool_or_skip, seed};
use super::*;

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
                      ('2020-06-16', $1, 2020, 6, 16, 102, 102, 102, 102),
                      ('2020-06-17', $1, 2020, 6, 17, 101, 101, 101, 101),
                      ('2020-06-30', $1, 2020, 6, 30, 103, 103, 103, 103)"#,
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
    assert_eq!(june["fill_target"], 104.76, "配股 0.5 元：110 ÷ 1.05");
    assert_eq!(june["filled"], false);
    assert!(june["fill_date"].is_null());
    assert_eq!(june["trading_days_since_ex"], 3);
    assert_eq!(june["latest_close"], 103.0);
    assert_eq!(june["gap_percent"], 1.68, "(104.76 − 103) ÷ 104.76");

    let march = &events[1];
    assert_eq!(
        march["cash_dividend"], 2.5,
        "同日的彙總列與月別重複列都不重複計算"
    );
    assert_eq!(march["filled"], true);
    assert_eq!(march["fill_date"], "2020-03-19");
    assert_eq!(march["trading_days_to_fill"], 3);
    assert_eq!(march["fill_target"], 100.0, "沒有配股時就是前一日收盤");
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
