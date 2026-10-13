//! 資料庫語意整合測試的共用工具：帶金鑰的 GET、連線池、假代號的種入與清理。

use super::*;

/// 假代號；跑完會清掉所有種入的列。
pub(super) const SYMBOL: &str = "79979DA";

/// 回傳可用的 API 金鑰；測試環境沒設定時自行補一組（`--test-threads=1` 下無競爭）。
fn api_key() -> String {
    std::env::var("DATA_API_KEY").unwrap_or_else(|_| {
        let generated = "stocks-integration-test-key".to_owned();
        unsafe { std::env::set_var("DATA_API_KEY", &generated) };
        generated
    })
}

/// 以真實 router 發出帶金鑰的 GET，回傳狀態碼與 JSON body。
pub(super) async fn get(path: &str) -> (StatusCode, serde_json::Value) {
    let response = router()
        .oneshot(
            Request::get(path)
                .header("Authorization", format!("Bearer {}", api_key()))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should serve request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    (
        status,
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
    )
}

/// 資料庫可用時回傳連線池，否則印出跳過訊息。
pub(super) async fn pool_or_skip(test: &str) -> Option<&'static sqlx::PgPool> {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 {test}：無資料庫連接");
        return None;
    }
    Some(pool)
}

/// 刪除假代號在各表的資料。
pub(super) async fn cleanup(pool: &sqlx::PgPool) {
    for sql in [
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = $1"#,
        "DELETE FROM last_daily_quotes WHERE stock_symbol = $1",
        "DELETE FROM quote_history_record WHERE security_code = $1",
        r#"DELETE FROM "Revenue" WHERE "SecurityCode" = $1"#,
        "DELETE FROM financial_statement WHERE security_code = $1",
        "DELETE FROM dividend WHERE security_code = $1",
        "DELETE FROM estimate WHERE security_code = $1",
        "DELETE FROM income_statement WHERE stock_symbol = $1",
        "DELETE FROM balance_sheet WHERE stock_symbol = $1",
        "DELETE FROM cash_flow_statement WHERE stock_symbol = $1",
        "DELETE FROM financial_health WHERE stock_symbol = $1",
        "DELETE FROM stocks WHERE stock_symbol = $1",
    ] {
        let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
    }
}

/// 依序執行種入資料的 SQL（全部只綁 `$1 = SYMBOL`）。
pub(super) async fn seed(pool: &sqlx::PgPool, statements: &[&'static str]) {
    for &sql in statements {
        sqlx::query(sql)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("種入測試資料失敗：{error}\n{sql}"));
    }
}

/// 股票主檔一列；兩組測試共用。
pub(super) const SEED_STOCK: &str = r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id, "SuspendListing", last_one_eps, last_four_eps, net_asset_value_per_share, return_on_equity, weight, issued_share)
    VALUES ($1, '端點測試股', $1, 2, 1, false, 1.25, 5.5, 30.1, 18.2, 0.01, 1000000)"#;
