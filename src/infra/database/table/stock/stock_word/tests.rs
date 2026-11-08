//! 股票搜尋字詞表的測試。

use crate::core::util;

use super::*;

/*    #[tokio::test]
    async fn test_vec_to_hashmap() {
        dotenvy::dotenv().ok();
        let mut entities: Vec<StockWord> = Vec::new();
        for i in 0..1000000 {
            entities.push(StockWord {
                word_id: 0,
                word: format!("word_{}", i),
                created_time: Default::default(),
                updated_time: Default::default(),
            });
        }

        let start1 = Instant::now();
        let _hm1 = vec_to_hashmap_key_using_word(Some(entities.clone()));
        let elapsed1 = start1.elapsed().as_millis();

        /*let start2 = Instant::now();
        let hm2 = vec_to_hashmap(Some(entities.clone()));
        let elapsed2 = start2.elapsed().as_millis();*/

        println!("Method 1 elapsed time: {}", elapsed1);
        //println!("Method 2 elapsed time: {}", elapsed2);
        //println!("HashMap length: {} {}", hm1.len(), hm2.len());
    }
*/
/*    #[tokio::test]
    async fn test_split_1() {
        dotenvy::dotenv().ok();
        let chinese_word = "台積電";
        let start = Instant::now();
        let result = split_v1(chinese_word);
        let end = start.elapsed();
        println!("split: {:?}, elapsed time: {:?}", result, end);
    }
*/

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_insert() {
    dotenvy::dotenv().ok();
    let mut e = StockWord::new("小一".to_string());
    match e.upsert().await {
        Ok(word_id) => {
            tracing::debug!("word_id:{} e:{:#?}", word_id, &e);
            let _ = sqlx::query("delete from company_word where word_id = $1;")
                .bind(word_id)
                .execute(database::get_connection())
                .await;
        }
        Err(why) => {
            tracing::debug!("because:{:?}", why);
        }
    }
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_list_by_word() {
    dotenvy::dotenv().ok();
    let word = util::text::split("隆銘綠能");
    let entities = StockWord::list_by_word(&word).await;
    tracing::debug!("entities:{:#?}", entities);
    /*tracing::debug!("word:{:#?}",
    util::map::vec_to_hashmap(entities.unwrap()));*/
}
