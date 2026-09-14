//! 唯讀查詢小工具：把任意 SELECT 包成 JSON 後印出。
//!
//! 用法：cargo run --example dbq -- "SELECT 1 AS n"
//!
//! 只開唯讀交易，任何寫入都會被資料庫擋下，因此可以直接查正式庫做診斷。
//!
//! SQL 是直接拼進查詢的（唯讀交易是唯一的防線），只給維運人員從本機手動執行，
//! 不要把任何外部輸入接進來，也不要在服務流程裡呼叫。
use anyhow::{Result, anyhow};
use sqlx::{Row, postgres::PgPoolOptions};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    // 參數以 @ 開頭時視為檔案路徑：SQL 含 || 或 ~ 時，用檔案比塞進命令列安全。
    let arg = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("需要一段 SQL 或 @檔案路徑"))?;
    let sql = match arg.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path)?,
        None => arg,
    };

    let url = std::env::var("DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?;

    // 明確開唯讀交易，避免這支工具有任何寫入的可能。
    let mut tx = pool.begin().await?;
    sqlx::raw_sql("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await?;

    let wrapped = format!(
        "SELECT COALESCE(json_agg(t)::text, '[]') AS payload FROM ({}) AS t",
        sql.trim_end().trim_end_matches(';')
    );
    // sqlx 0.9 的 SqlSafeStr 只接受 'static 字串，動態組出來的 SQL 這裡直接洩漏成 'static。
    // 這是一次性的查詢工具，行程結束就回收，不值得為此繞路。
    let wrapped: &'static str = Box::leak(wrapped.into_boxed_str());
    let row = sqlx::raw_sql(wrapped).fetch_one(&mut *tx).await?;
    let payload: String = row.try_get("payload")?;

    tx.rollback().await?;
    println!("{payload}");

    Ok(())
}
