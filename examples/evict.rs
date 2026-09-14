//! 清掉指定的 Redis 快取 key，讓剛改過的資料能立刻反映。
//!
//! 用法：cargo run --example evict -- "Dividend:2753" "yahoo:dividend:2753"
//!
//! 連線設定沿用 .env，密碼不會出現在參數或輸出。
//!
//! 手動改過資料庫之後用它清掉對應的快取，前端才不用等 TTL 到期（Go 端的股利明細快取 1 小時）。
use anyhow::Result;
use stock_crawler::infra::nosql::redis::CLIENT;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let keys: Vec<String> = std::env::args().skip(1).collect();
    if keys.is_empty() {
        println!("需要至少一個 key");
        return Ok(());
    }

    for key in keys {
        // 先看存不存在，才知道刪除是真的清掉還是本來就沒有。
        let existed = CLIENT.contains_key(&key).await.unwrap_or(false);
        match CLIENT.delete(&key).await {
            Ok(_) => println!("{key}  existed={existed}  deleted"),
            Err(why) => println!("{key}  existed={existed}  failed: {why}"),
        }
    }

    Ok(())
}
