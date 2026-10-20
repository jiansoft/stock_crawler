//! 連續天數在 Redis 的存取（需要 Redis）。

use super::*;

/// 連續天數寫入 Redis 後讀得回來；測試用假代號，結束時刪除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn streak_round_trips_through_redis() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 streak_round_trips_through_redis：無 Redis 連線");
        return;
    }
    let symbol = "79975";
    let _ = CLIENT.delete(&streak_key(symbol)).await;
    assert_eq!(load_streak(symbol).await, None);

    let streak = Streak {
        date: day(6),
        foreign: 4,
        trust: -2,
    };
    save_streak(symbol, &streak).await;
    assert_eq!(load_streak(symbol).await, Some(streak));

    CLIENT
        .delete(&streak_key(symbol))
        .await
        .expect("清除測試紀錄");
}
