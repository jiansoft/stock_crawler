//! 條件選股回測 endpoint 的資料庫語意整合測試。

mod attribution;
mod compare;
mod cutoff;
mod etf_gap;
mod execution;
mod filters;
mod fundamentals;
mod major;
mod margins;
mod offsets;
mod periods;
mod presets;
mod ranking;
mod stops;
mod stops_next_open;
mod track;
mod valuation;
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
        "DELETE FROM holder_distribution WHERE stock_symbol = ANY($1)",
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
    let _ = sqlx::query(
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol='0050' AND "Date" BETWEEN $1 AND $2"#,
    )
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
    // 每個交易日都有三檔的分數（預設 50），換股日用 SCORES 的分數與名次。
    for d in DAYS {
        for (symbol, _, _) in STOCKS {
            let score = SCORES
                .iter()
                .find(|row| row.0 == d && row.1 == symbol)
                .map_or(50, |row| row.2);
            let rank: i32 = match (d, symbol) {
                (1, "79979BA") => 1,
                (1, "79979BC") => 2,
                (1, "79979BB") => 3,
                (8, "79979BB") => 1,
                (8, "79979BC") => 2,
                (8, "79979BA") => 3,
                _ => 1,
            };
            sqlx::query(
                "INSERT INTO daily_relative_strength (date, stock_symbol, weighted_return, rs_score, rank, universe)
                 VALUES ($1, $2, 0, $3, $4, 3)",
            )
            .bind(day(d))
            .bind(symbol)
            .bind(score)
            .bind(rank)
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
