//! 股票搜尋索引的測試。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_insert() {
    dotenvy::dotenv().ok();
    let mut e = StockIndex::new("79979".to_string());
    e.word_id = 79979;
    match e.insert().await {
        Ok(_) => {
            match sqlx::query_as::<sqlx::Postgres, (i64, )>("select count(*) as row_count from company_index where word_id = $1 and security_code = $2;")
                .bind(e.word_id)
                .bind(e.security_code.as_str())
                .fetch_one(database::get_connection())
                .await
            {
                Ok((row_count, )) => {
                    tracing::info!("row_count:{}", row_count);
                    let _ = sqlx::query(
                        "delete from company_index where word_id = $1 and security_code = $2;",
                    )
                        .bind(e.word_id)
                        .bind(e.security_code.as_str())
                        .execute(database::get_connection())
                        .await;
                }
                Err(why) => {
                    tracing::error!("because:{:#?}", why);
                }
            };
        }
        Err(why) => {
            tracing::error!("because:{:#?}", why);
        }
    }
}
