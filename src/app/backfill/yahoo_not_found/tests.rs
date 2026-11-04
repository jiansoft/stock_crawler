//! Yahoo 404 略過時間的測試：判斷規則（純函式）與 Redis 標記流程（整合測試）。

use super::*;

/// 第一次 404 沿用一般略過時間；標記期間內再次 404 才拉長到 30 天。
#[test]
fn decide_extends_only_repeated_not_found() {
    assert_eq!(
        decide(true, 259_200),
        NotFoundSkip {
            ttl_seconds: 259_200,
            repeated: false,
        }
    );
    assert_eq!(
        decide(false, 259_200),
        NotFoundSkip {
            ttl_seconds: PAGE_NOT_FOUND_CACHE_TTL_SECONDS,
            repeated: true,
        }
    );
    assert_eq!(
        marker_key("yahoo:dividend:2330"),
        "yahoo:dividend:2330:not_found"
    );
}

/// 第一次 404 記標記、第二次拉長；成功後清掉標記，下一次 404 又算第一次。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn on_not_found_counts_repeats_until_success() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 on_not_found_counts_repeats_until_success：無 Redis 連接");
        return;
    }
    let cache_key = "yahoo:test:79979Y";
    let _ = CLIENT.delete(&marker_key(cache_key)).await;

    let first = on_not_found(cache_key, 100).await;
    let second = on_not_found(cache_key, 100).await;
    on_success(cache_key).await;
    let after_success = on_not_found(cache_key, 100).await;
    let _ = CLIENT.delete(&marker_key(cache_key)).await;

    assert!(!first.repeated && first.ttl_seconds == 100, "{first:?}");
    assert!(
        second.repeated && second.ttl_seconds == PAGE_NOT_FOUND_CACHE_TTL_SECONDS,
        "{second:?}"
    );
    assert!(!after_success.repeated, "成功後重新計算：{after_success:?}");
}
