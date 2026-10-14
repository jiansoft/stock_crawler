//! 法人買賣超 endpoint 的資料庫語意整合測試。

use axum::http::StatusCode;

use super::support::{get, pool_or_skip};

/// `(代號, 市場, 產業)`；產業 24 為 CI 種入的「半導體業」，79979FC 的 0 為未分類。
const STOCKS: [(&str, i32, i32); 3] = [("79979FA", 2, 24), ("79979FB", 4, 24), ("79979FC", 2, 0)];

async fn cleanup(pool: &sqlx::PgPool) {
    let symbols: Vec<&str> = STOCKS.iter().map(|row| row.0).collect();
    for sql in [
        "DELETE FROM chip_daily WHERE stock_symbol = ANY($1)",
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(&symbols).execute(pool).await;
    }
}

/// 1988-03-01～03 三個交易日：
/// - FA：外資 −1000、+2000、+3000，投信 0、+1000、−1000，自營商 0、0、+500；收盤 10、成交 10000 股。
/// - FB：外資每天 +5000；收盤 1、成交 100000 股。
/// - FC：只有 03-03，外資 −4000；收盤 20、成交 8000 股。
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
    for sql in [
        r#"INSERT INTO chip_daily ("date", stock_symbol, foreign_net, trust_net, dealer_net) VALUES
           ('1988-03-01', '79979FA', -1000, 0, 0), ('1988-03-02', '79979FA', 2000, 1000, 0),
           ('1988-03-03', '79979FA', 3000, -1000, 500),
           ('1988-03-01', '79979FB', 5000, 0, 0), ('1988-03-02', '79979FB', 5000, 0, 0),
           ('1988-03-03', '79979FB', 5000, 0, 0),
           ('1988-03-03', '79979FC', -4000, 0, 0)"#,
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "TradingVolume")
           SELECT d, '79979FA', 10, 10000 FROM UNNEST('{1988-03-01,1988-03-02,1988-03-03}'::date[]) AS d
           UNION ALL
           SELECT d, '79979FB', 1, 100000 FROM UNNEST('{1988-03-01,1988-03-02,1988-03-03}'::date[]) AS d
           UNION ALL
           SELECT '1988-03-03', '79979FC', 20, 8000"#,
    ] {
        sqlx::query(sql)
            .execute(pool)
            .await
            .expect("種入籌碼與日線");
    }
}

/// 回應中各檔的 `(代號, 名次)`。
fn ranked(json: &serde_json::Value) -> Vec<(String, u64)> {
    json["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|item| {
            (
                item["stock_symbol"].as_str().unwrap_or_default().to_owned(),
                item["rank"].as_u64().unwrap_or_default(),
            )
        })
        .collect()
}

/// 累計期間、金額排名、成交量占比、連續天數、方向／市場／產業篩選、參數驗證與查無資料。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn institutional_flow_endpoint_db_semantics() {
    let Some(pool) = pool_or_skip("institutional_flow_endpoint_db_semantics").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool).await;

    // 1988-03-05 是週六，資料日回推到 03-03；外資近 2 日：FA 5000 股 × 10 元、FB 10000 股 × 1 元。
    let (status, json) =
        get("/api/v1/market/institutional-flow?date=1988-03-05&days=2&limit=100").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["data_as_of"], "1988-03-03");
    assert_eq!(json["from"], "1988-03-02");
    assert_eq!(json["days"], 2);
    assert_eq!(json["investor"], "foreign");
    assert_eq!(json["side"], "buy");
    assert_eq!(json["market"], "all");
    assert!(json["industry"].is_null());
    assert_eq!(
        ranked(&json),
        [("79979FA".to_owned(), 1), ("79979FB".to_owned(), 2)]
    );
    let fa = &json["items"][0];
    assert_eq!(fa["name"], "79979FA");
    assert_eq!(fa["market"], "twse");
    assert_eq!(fa["industry"], "半導體業");
    assert_eq!(fa["net_shares"], 5000);
    assert_eq!(fa["net_lots"], 5);
    assert_eq!(fa["net_amount"], 50_000.0);
    assert_eq!(fa["volume_ratio"], 25.0);
    assert_eq!(
        (fa["buy_days"].as_i64(), fa["sell_days"].as_i64()),
        (Some(2), Some(0))
    );
    assert_eq!(fa["streak"], 2, "03-01 賣超中斷");
    assert_eq!(fa["closing_price"], 10.0);
    let fb = &json["items"][1];
    assert_eq!(fb["market"], "tpex");
    assert_eq!(fb["streak"], 3, "回看範圍內每天都買超");
    assert_eq!(fb["volume_ratio"], 5.0);

    // 賣超：FC 外資 −4000 × 20 元；連續賣超 1 天；未分類。
    let (status, json) =
        get("/api/v1/market/institutional-flow?date=1988-03-03&side=sell&limit=100").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(ranked(&json), [("79979FC".to_owned(), 1)]);
    assert_eq!(json["items"][0]["net_amount"], -80_000.0);
    assert_eq!(json["items"][0]["streak"], -1);
    assert_eq!(json["items"][0]["industry"], "");

    // 合計：FA 3000 − 1000 + 500 = 2500 股；投信 3 日 FA 淨 0 不列入買超、資料日賣超 1000 股。
    let (_, json) =
        get("/api/v1/market/institutional-flow?date=1988-03-03&investor=total&limit=100").await;
    assert_eq!(json["items"][0]["stock_symbol"], "79979FA");
    assert_eq!(json["items"][0]["net_shares"], 2500);
    let (_, json) =
        get("/api/v1/market/institutional-flow?date=1988-03-03&investor=trust&days=3&limit=100")
            .await;
    assert!(ranked(&json).iter().all(|(symbol, _)| symbol != "79979FA"));
    let (_, json) =
        get("/api/v1/market/institutional-flow?date=1988-03-03&investor=trust&side=sell&limit=100")
            .await;
    assert_eq!(json["items"][0]["stock_symbol"], "79979FA");
    assert_eq!(json["items"][0]["streak"], -1);

    // 市場、產業篩選與 limit。
    let (_, json) =
        get("/api/v1/market/institutional-flow?date=1988-03-03&market=tpex&limit=100").await;
    assert_eq!(ranked(&json), [("79979FB".to_owned(), 1)]);
    let (_, json) = get(
        "/api/v1/market/institutional-flow?date=1988-03-03&industry=%E5%8D%8A%E5%B0%8E%E9%AB%94%E6%A5%AD&limit=1",
    )
    .await;
    assert_eq!(json["industry"], "半導體業");
    assert_eq!(json["matched"], 2);
    assert_eq!(ranked(&json), [("79979FA".to_owned(), 1)]);

    for query in [
        "days=0",
        "days=61",
        "investor=retail",
        "side=hold",
        "market=otc",
        "limit=0",
        "limit=101",
        "date=1988-3-3",
        "industry=%E4%B8%8D%E5%AD%98%E5%9C%A8%E7%94%A2%E6%A5%AD",
    ] {
        let (status, _) = get(&format!("/api/v1/market/institutional-flow?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }
    let (status, _) = get("/api/v1/market/institutional-flow?date=1900-01-01").await;
    cleanup(pool).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
