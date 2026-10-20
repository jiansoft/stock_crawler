//! 條件選股回測 endpoint 的資料庫語意整合測試。

mod attribution;
mod filters;
mod fundamentals;
mod margins;
mod offsets;
mod presets;
mod ranking;
mod stops;
mod track;
mod walk_forward;

use axum::http::StatusCode;
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::support::{get, pool_or_skip};

/// `(代號, 市場, 產業)`；產業 24 為 CI 種入的「半導體業」。
const STOCKS: [(&str, i32, i32); 3] = [("79979BA", 2, 24), ("79979BB", 2, 99), ("79979BC", 4, 24)];

/// 有相對強度的交易日（1993-03-dd）：持有 5 日時第 1、6 天換股，第 11 天還沒有報酬。
const DAYS: [u32; 11] = [1, 2, 3, 4, 5, 8, 9, 10, 11, 12, 15];

/// `(日, 代號, 相對強度分數, 5 日報酬)`；03-08 的 BC 沒有報酬（之後完全沒有收盤）。
const SCORES: [(u32, &str, i16, Option<i64>); 6] = [
    (1, "79979BA", 90, Some(10)),
    (1, "79979BB", 60, Some(-4)),
    (1, "79979BC", 80, Some(2)),
    (8, "79979BA", 50, Some(3)),
    (8, "79979BB", 95, Some(8)),
    (8, "79979BC", 70, None),
];

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(1993, 3, d).expect("固定日期合法")
}

/// 刪除種入的相對強度、報酬、訊號、籌碼、月營收、季報、三大財報與股票主檔。
async fn cleanup(pool: &sqlx::PgPool) {
    let symbols: Vec<&str> = STOCKS.iter().map(|row| row.0).collect();
    for sql in [
        "DELETE FROM daily_relative_strength WHERE stock_symbol = ANY($1)",
        "DELETE FROM daily_forward_return WHERE stock_symbol = ANY($1)",
        "DELETE FROM daily_technical_signal WHERE stock_symbol = ANY($1)",
        "DELETE FROM chip_daily WHERE stock_symbol = ANY($1)",
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
        r#"DELETE FROM "Revenue" WHERE "SecurityCode" = ANY($1)"#,
        "DELETE FROM financial_statement WHERE security_code = ANY($1)",
        "DELETE FROM income_statement WHERE stock_symbol = ANY($1)",
        "DELETE FROM balance_sheet WHERE stock_symbol = ANY($1)",
        "DELETE FROM cash_flow_statement WHERE stock_symbol = ANY($1)",
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(&symbols).execute(pool).await;
    }
    let _ =
        sqlx::query(r#"DELETE FROM "index" WHERE category = 'TAIEX' AND "date" BETWEEN $1 AND $2"#)
            .bind(day(1))
            .bind(day(31))
            .execute(pool)
            .await;
    let _ = sqlx::query("DELETE FROM daily_stock_price_stats WHERE date BETWEEN $1 AND $2")
        .bind(day(1))
        .bind(day(31))
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM daily_forward_return WHERE stock_symbol = '0050' AND date BETWEEN $1 AND $2",
    )
    .bind(day(1))
    .bind(day(31))
    .execute(pool)
    .await;
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
    // 每個交易日都有三檔的分數（預設 50），換股日用 SCORES 的分數。
    for d in DAYS {
        for (symbol, _, _) in STOCKS {
            let score = SCORES
                .iter()
                .find(|row| row.0 == d && row.1 == symbol)
                .map_or(50, |row| row.2);
            sqlx::query(
                "INSERT INTO daily_relative_strength (date, stock_symbol, weighted_return, rs_score, rank, universe)
                 VALUES ($1, $2, 0, $3, 1, 3)",
            )
            .bind(day(d))
            .bind(symbol)
            .bind(score)
            .execute(pool)
            .await
            .expect("種入相對強度");
        }
    }
    for (d, symbol, _, return_5d) in SCORES {
        sqlx::query(
            "INSERT INTO daily_forward_return (date, stock_symbol, return_5d) VALUES ($1, $2, $3)",
        )
        .bind(day(d))
        .bind(symbol)
        .bind(return_5d.map(Decimal::from))
        .execute(pool)
        .await
        .expect("種入報酬");
    }
    // 加權指數：03-01～03-05 為 100，03-08 跌到 90，低於近 6 日平均（約 98.3）。
    for (d, index) in [(1, 100), (2, 100), (3, 100), (4, 100), (5, 100), (8, 90)] {
        sqlx::query(r#"INSERT INTO "index" (category, "date", "index") VALUES ('TAIEX', $1, $2)"#)
            .bind(day(d))
            .bind(Decimal::from(index))
            .execute(pool)
            .await
            .expect("種入加權指數");
    }
    // 市場廣度（上市櫃合併）：03-01 站上 60 日線 70%、03-08 只剩 30%；上櫃（id 4）只有 03-01 的 40%。
    for (d, market_id, above, below) in [(1, 0, 70, 30), (8, 0, 30, 70), (1, 4, 40, 60)] {
        sqlx::query(
            "INSERT INTO daily_stock_price_stats
                 (date, stock_exchange_market_id, above_60_day_moving_average, below_60_day_moving_average)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(day(d))
        .bind(market_id)
        .bind(above)
        .bind(below)
        .execute(pool)
        .await
        .expect("種入市場廣度");
    }
    // 0050：03-01 買進 5 日報酬 4%；03-08 沒有資料（彙總時算 0）。
    sqlx::query(
        "INSERT INTO daily_forward_return (date, stock_symbol, return_5d) VALUES ($1, '0050', 4)",
    )
    .bind(day(1))
    .execute(pool)
    .await
    .expect("種入 0050 報酬");
    // BB 在 03-03 出現 KD 黃金交叉（落在 03-08 換股的回看視窗 03-02～03-08 內）。
    sqlx::query(
        "INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
         VALUES ($1, '79979BB', 'kd', 'golden_cross', 'KD 黃金交叉')",
    )
    .bind(day(3))
    .execute(pool)
    .await
    .expect("種入訊號");
    // 外資：BA 02-26、03-01 連買 2 天；BB 03-05、03-08 連買 2 天。投信：BC 03-08 買超 1 天。
    sqlx::query(
        r#"INSERT INTO chip_daily ("date", stock_symbol, foreign_net, trust_net, dealer_net) VALUES
           ('1993-02-26', '79979BA', 100, 0, 0), ('1993-03-01', '79979BA', 100, 0, 0),
           ('1993-03-05', '79979BB', 100, 0, 0), ('1993-03-08', '79979BB', 100, 0, 0),
           ('1993-03-08', '79979BC', 0, 100, 0)"#,
    )
    .execute(pool)
    .await
    .expect("種入籌碼");
    crate::infra::database::repository::chip::refresh_streaks_for_tests().await;
    // 成交金額：BA 只有 03-01 的 3,000 萬元、BC 03-01 有 2 億元；BB 沒有日線。
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "TradeValue") VALUES
           ('1993-03-01', '79979BA', 30000000), ('1993-03-01', '79979BC', 200000000)"#,
    )
    .execute(pool)
    .await
    .expect("種入日線成交金額");
}

/// 呼叫 endpoint（固定 03-01～03-15、持有 5 日），回傳狀態與 JSON。
async fn query(extra: &str) -> (StatusCode, serde_json::Value) {
    get(&format!(
        "/api/v1/stocks/screen/backtest?from=1993-03-01&to=1993-03-15&hold_days=5&{extra}"
    ))
    .await
}

/// 每期 `(日期, 檔數, 平均報酬)`。
fn periods(json: &serde_json::Value) -> Vec<(String, i64, Option<f64>)> {
    json["periods"]
        .as_array()
        .expect("periods array")
        .iter()
        .map(|period| {
            (
                period["date"].as_str().unwrap_or_default().to_owned(),
                period["holdings"].as_i64().unwrap_or_default(),
                period["avg_return"].as_f64(),
            )
        })
        .collect()
}

/// 取得資料庫連線並重新種入測試資料；沒有資料庫時回 `None`（測試略過）。
async fn seeded(test: &str) -> Option<&'static sqlx::PgPool> {
    let pool = pool_or_skip(test).await?;
    cleanup(pool).await;
    seed(pool).await;
    Some(pool)
}

/// 換股日、相對強度排序取前 N 檔、沒有報酬的股票不列入平均、大盤基準、0050、年度彙總、交易成本、換股緩衝、
/// 續抱條件、產業上限、個股貢獻、目前持股與換股日偏移。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_periods_costs_and_current() {
    let Some(pool) = seeded("screen_backtest_periods_costs_and_current").await else {
        return;
    };
    // 每期取相對強度前 2 檔：03-01 BA、BC（10、2），03-08 BB、BC（BC 沒有報酬，只算 BB 的 8）。
    // 03-15 還沒有報酬，不列入。
    let (status, json) = query("limit=2").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["to"], "1993-03-15");
    assert_eq!(
        (json["hold_days"].as_u64(), json["limit"].as_u64()),
        (Some(5), Some(2))
    );
    assert_eq!(json["adjusted"], true);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(6.0)),
            ("1993-03-08".to_owned(), 2, Some(8.0)),
        ]
    );
    assert_eq!(
        json["periods"][0]["symbols"],
        serde_json::json!(["79979BA", "79979BC"])
    );
    // 大盤基準：03-01 三檔平均 8/3；03-08 只有兩檔有報酬，平均 5.5。
    assert_eq!(json["periods"][0]["benchmark_return"], 2.67);
    assert_eq!(json["periods"][1]["benchmark_return"], 5.5);
    let summary = &json["summary"];
    assert_eq!(summary["periods"], 2);
    assert_eq!(summary["invested_periods"], 2);
    assert_eq!(summary["total_return"], 14.48);
    assert_eq!(summary["benchmark_total_return"], 8.31);
    assert_eq!(summary["win_rate"], 1.0);
    assert_eq!(summary["beat_market_rate"], 1.0);
    assert_eq!(summary["max_drawdown"], 0.0);
    // 0050：03-01 報酬 4%、03-08 缺值算 0；兩期都勝過 0050。
    assert_eq!(json["etf_benchmark"], "0050");
    assert_eq!(json["periods"][0]["etf_return"], 4.0);
    assert!(json["periods"][1]["etf_return"].is_null());
    assert_eq!(summary["etf_total_return"], 4.0);
    assert_eq!(summary["etf_max_drawdown"], 0.0);
    assert_eq!(summary["beat_etf_rate"], 1.0);
    // 年度彙總：兩期都在 1993 年。
    let years = json["years"].as_array().expect("years array");
    assert_eq!(years.len(), 1);
    assert_eq!(
        (years[0]["year"].as_i64(), years[0]["periods"].as_i64()),
        (Some(1993), Some(2))
    );
    assert_eq!(years[0]["total_return"], 14.48);
    assert_eq!(years[0]["benchmark_total_return"], 8.31);
    assert_eq!(years[0]["etf_total_return"], 4.0);
    // 交易成本：03-01 全部買進（0.1425%）；03-01 那期 BA 漲 10%、BC 漲 2%，期末 BA 占 0.55 ÷ 1.06。
    // 03-08 賣出 BA（0.4425%）、BC 加碼回一半與買進 BB 一半（0.1425%），兩邊都是 0.55 ÷ 1.06。
    assert_eq!(json["fee_discount"], 1.0);
    assert_eq!(json["periods"][0]["turnover"], 1.0);
    assert_eq!(json["periods"][0]["return_after_costs"], 5.86);
    assert_eq!(json["periods"][1]["turnover"], 0.5189);
    // 8 − 0.5189 × (0.4425 + 0.1425) ≈ 7.70。
    assert_eq!(json["periods"][1]["return_after_costs"], 7.7);
    assert_eq!(summary["avg_turnover"], 0.7594);
    // 1.058575 × 1.076965 − 1 ≈ 14.00%。
    assert_eq!(summary["total_return_after_costs"], 14.0);
    let (_, free) = query("limit=2&fee_discount=0").await;
    assert_eq!(free["periods"][0]["return_after_costs"], 6.0);
    // 只剩賣出 BA 的證交稅：8 − 0.5189 × 0.3。
    assert_eq!(free["periods"][1]["return_after_costs"], 7.84);

    // 換股緩衝：只持 1 檔、緩衝 3 名。03-08 的 BA 掉到第 3 名仍續抱（報酬 3），不換成第 1 名的 BB。
    let (status, json) = query("limit=1&keep_rank=3").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["keep_rank"], 3);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(3.0)),
        ]
    );
    assert_eq!(
        json["periods"][1]["symbols"],
        serde_json::json!(["79979BA"])
    );
    assert_eq!(json["periods"][1]["turnover"], 0.0);
    // 緩衝 2 名時 BA（第 3 名）賣出、換成 BB。
    let (_, json) = query("limit=1&keep_rank=2").await;
    assert_eq!(
        json["periods"][1]["symbols"],
        serde_json::json!(["79979BB"])
    );
    // 只有兩期，報酬有波動才有夏普值。
    assert!(json["summary"]["volatility"].as_f64().is_some());
    assert!(json["summary"]["sharpe_ratio"].as_f64().is_some());

    // 續抱條件：只選 RS ≥ 85（03-01 BA、03-08 BB）。BA 03-08 的 RS 50 不再符合選股條件，
    // 但達續抱門檻 50 就續抱（報酬 3）；門檻 51 時換成 BB（報酬 8）。
    let (status, json) = query("limit=1&min_rs_score=85&hold_min_rs_score=50").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["hold_min_rs_score"], 50);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(3.0)),
        ]
    );
    let (_, json) = query("limit=1&min_rs_score=85&hold_min_rs_score=51").await;
    assert_eq!(
        json["periods"][1]["symbols"],
        serde_json::json!(["79979BB"])
    );

    // 產業上限：03-01 前兩名 BA（90）、BC（80）同為產業 24，上限 1 時跳過 BC、補產業 99 的 BB；
    // 03-08 的 BB（95，產業 99）、BC（70，產業 24）本來就不同產業。
    let (status, json) = query("limit=2&max_per_industry=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["max_per_industry"], 1);
    assert_eq!(
        json["periods"][0]["symbols"],
        serde_json::json!(["79979BA", "79979BB"])
    );
    assert_eq!(json["periods"][0]["avg_return"], 3.0);
    assert_eq!(
        json["periods"][1]["symbols"],
        serde_json::json!(["79979BB", "79979BC"])
    );
    let (_, json) = query("limit=2").await;
    assert!(json["max_per_industry"].is_null());

    // 個股貢獻：limit=2 時 03-01 BA 10、BC 2（各 1/2）、03-08 BB 8、BC 沒有報酬（BB 獨得）。
    let (_, json) = query("limit=2").await;
    let contributors = &json["contributors"];
    assert_eq!(contributors["held_symbols"], 3);
    assert_eq!(
        contributors["best"],
        serde_json::json!([
            {"symbol": "79979BB", "name": "79979BB", "periods_held": 1, "contribution": 8.0, "avg_return": 8.0},
            {"symbol": "79979BA", "name": "79979BA", "periods_held": 1, "contribution": 5.0, "avg_return": 10.0},
            {"symbol": "79979BC", "name": "79979BC", "periods_held": 2, "contribution": 1.0, "avg_return": 2.0},
        ])
    );
    assert_eq!(contributors["worst"], serde_json::json!([]));

    // 目前持股：03-15 還沒滿 5 日（不列入各期），三檔分數都是 50、依代號取 BA、BB；
    // 上一期的 BB 續抱、BA 新買進（換手 0.5），上一期的 BC 賣出。
    assert_eq!(
        json["current"],
        serde_json::json!({
            "date": "1993-03-15", "days_elapsed": 0, "days_to_next_rebalance": 5,
            "market_filtered": false, "turnover": 0.5, "cash": 0.0,
            // 種入的日線沒有收盤價，換股日至今的報酬都是 null。
            "return_to_date": null, "etf_return_to_date": null,
            "holdings": [
                {"symbol": "79979BA", "name": "79979BA", "weight": 50.0, "added": true, "return_to_date": null},
                {"symbol": "79979BB", "name": "79979BB", "weight": 50.0, "added": false, "return_to_date": null},
            ],
            "exited": [{"symbol": "79979BC", "name": "79979BC", "weight": 50.0}],
        })
    );
    // 迄日 03-12：最後換股日 03-08 已有報酬，沒有目前持股。
    let (status, json) =
        get("/api/v1/stocks/screen/backtest?from=1993-03-01&to=1993-03-12&hold_days=5&limit=2")
            .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["periods"].as_array().map(Vec::len), Some(2));
    assert!(json["current"].is_null());

    // 換股日偏移 1：換股日變成 03-02、03-09，都沒有種入報酬；03-02 不列入，03-09 是目前持股，
    // 之後經過 03-10、11、12、15 四個交易日，再 1 天換股。
    let (status, json) = query("limit=2&rebalance_offset=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["rebalance_offset"], 1);
    assert_eq!(periods(&json), []);
    assert_eq!(json["current"]["date"], "1993-03-09");
    assert_eq!(json["current"]["days_elapsed"], 4);
    assert_eq!(json["current"]["days_to_next_rebalance"], 1);
    let (_, json) = query("limit=2").await;
    assert_eq!(json["rebalance_offset"], 0);
    let (status, _) = query("rebalance_offset=5").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    cleanup(pool).await;
}
