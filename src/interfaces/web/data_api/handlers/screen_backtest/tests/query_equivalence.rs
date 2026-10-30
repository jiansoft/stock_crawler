//! 在臨時庫比對成交金額物化前後的候選股、續抱與逐期報酬。

use std::future::Future;
use std::pin::Pin;

use super::super::portfolio::weigh;
use super::*;
use crate::infra::database;

/// 把 future 裝箱成 `dyn Future`，poll 改走間接呼叫。
///
/// 回測的 SQL future 很大；直接 `.await` 時 release 最佳化會把它的 poll 內聯進比對矩陣的每一層，
/// 單一 crate 的 release 測試編譯從約 2.5 分鐘暴增到 20 分鐘（CI 整合測試因此超過 30 分鐘被取消）。
/// `#[inline(never)]` 只擋得住回傳 future 的外層函式，擋不住 poll，所以改以型別抹除隔開。
fn boxed<'a, T>(future: impl Future<Output = T> + 'a) -> Pin<Box<dyn Future<Output = T> + 'a>> {
    Box::pin(future)
}

/// 籌碼門檻、排序、權重與換股日偏移，在兩種成交價模型下均保持一致。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要臨時 PostgreSQL")]
async fn materialized_liquidity_preserves_periods() {
    let pool = database::get_connection();
    let symbols = ["79979P1A", "79979P1B"];
    for symbol in symbols {
        sqlx::query(r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id) VALUES ($1, $1, $1, 2, 24)"#).bind(symbol).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO daily_relative_strength (date, stock_symbol, weighted_return, rs_score, rank, universe) SELECT d::date, $1, 1, 90, 1, 2 FROM generate_series('1992-02-01'::date, '1992-02-12', '1 day') d").bind(symbol).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO daily_forward_return (date, stock_symbol, return_5d) SELECT d::date, $1, 3 FROM generate_series('1992-02-01'::date, '1992-02-12', '1 day') d").bind(symbol).execute(pool).await.unwrap();
        sqlx::query(r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "OpeningPrice", "ClosingPrice", "TradeValue") SELECT d::date, $1, 10, 11, 1000000 FROM generate_series('1992-01-01'::date, '1992-02-12', '1 day') d"#).bind(symbol).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO chip_daily (date, stock_symbol, foreign_streak, trust_streak) SELECT d::date, $1, CASE WHEN $1 = '79979P1A' THEN 4 ELSE -1 END, 2 FROM generate_series('1992-02-01'::date, '1992-02-12', '1 day') d").bind(symbol).execute(pool).await.unwrap();
    }
    for execution in ["close", "next_open"] {
        for offset in [0, 1] {
            for weighting in ["equal", "trade_value", "sqrt_trade_value"] {
                boxed(compare_case(execution, offset, weighting)).await;
            }
        }
    }
    for table in [
        "chip_daily",
        "daily_forward_return",
        "daily_relative_strength",
        "\"DailyQuotes\"",
        "stocks",
    ] {
        // 表名只來自上面的固定清單；股票代號仍以參數綁定。
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM {table} WHERE stock_symbol = ANY($1)"
        )))
        .bind(symbols)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// 比對一組條件；每個 SQL future 都經 [`boxed`] 隔開，避免 release 最佳化把它們內聯進三層矩陣迴圈。
async fn compare_case(execution: &str, offset: u16, weighting: &str) {
    let params = serde_json::from_value(serde_json::json!({
        "from":"1992-02-01", "to":"1992-02-12", "hold_days":5,
        "limit":1,"keep_rank":2,"hold_min_rs_score":70,
        "min_foreign_streak":3,"rank_by":"trade_value",
        "weighting":weighting,"execution_price":execution
    }))
    .unwrap();
    let backtest = boxed(Backtest::prepare(&params))
        .await
        .unwrap_or_else(|_| panic!("條件應合法"))
        .period_end_only();
    let optimized = if execution == "close" {
        BACKTEST_WITH_CHIP_SQL
    } else {
        super::super::sql::BACKTEST_WITH_CHIP_NEXT_OPEN_SQL
    };
    let original = optimized.replace("valued AS MATERIALIZED (", "valued AS (");
    let before = boxed(backtest.candidates_using_sql(offset, 2, &original))
        .await
        .unwrap();
    let after = boxed(backtest.candidates(offset, 2)).await.unwrap();
    assert!(!after.is_empty());
    assert_eq!(before, after, "{execution} / {offset} / {weighting}");
    // 此回歸只改 SQL，沿用相同的純持股／權重／成本步驟驗證逐期結果。
    // 不重跑完整非同步日線組裝，避免兩個巨型 future 讓 release 最佳化膨脹。
    let mut before = select_holdings(before, 1, 2, None);
    let mut after = select_holdings(after, 1, 2, None);
    weigh(&mut before, backtest.weighting, None);
    weigh(&mut after, backtest.weighting, None);
    assert_eq!(before, after);
    let before_costs = trading_costs(&before, backtest.fee_discount);
    let after_costs = trading_costs(&after, backtest.fee_discount);
    for (before, after) in before_costs.iter().zip(&after_costs) {
        assert_eq!(before.net_return, after.net_return);
        assert_eq!(before.turnover, after.turnover);
    }
}

/// 續抱持股的平均成交金額補查：`liquidity_start`～換股日的平均（與選股時的算法相同），沒有日線的維持
/// `None`，已有值的不重查。
#[tokio::test]
#[cfg_attr(not(feature = "integration-tests"), ignore = "需要臨時 PostgreSQL")]
async fn hold_trade_values_are_filled_from_daily_quotes() {
    use super::super::engine::fill_hold_trade_values;
    let pool = database::get_connection();
    let symbol = "79979P2A";
    let first = NaiveDate::from_ymd_opt(1992, 3, 2).expect("有效日期");
    sqlx::query(r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "TradeValue") SELECT $2::date + n, $1, 10, 100 * (n + 1) FROM generate_series(0, 4) AS n"#)
        .bind(symbol)
        .bind(first)
        .execute(pool)
        .await
        .unwrap();
    let date = first + chrono::Days::new(4);
    let mut rows = vec![PeriodRow {
        date,
        symbols: vec![
            symbol.to_owned(),
            "79979P2Z".to_owned(),
            "79979P2Y".to_owned(),
        ],
        trade_values: vec![None, None, Some(1.0)],
        ..Default::default()
    }];
    let starts = std::collections::HashMap::from([(date, first + chrono::Days::new(2))]);

    let filled = fill_hold_trade_values(&mut rows, &starts).await;

    sqlx::query(r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#)
        .bind(symbol)
        .execute(pool)
        .await
        .unwrap();
    filled.unwrap();
    // 第 3～5 天：300、400、500 的平均。
    assert_eq!(rows[0].trade_values, [Some(400.0), None, Some(1.0)]);
}
