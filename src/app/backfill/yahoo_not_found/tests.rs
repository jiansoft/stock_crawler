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

/// 再次 404 時所有略過 key 都拉長；沒有 key 時不動 Redis。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn handle_not_found_extends_every_key_on_repeat() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 handle_not_found_extends_every_key_on_repeat：無 Redis 連接");
        return;
    }
    let keys = ["yahoo:test:79979Z", "yahoo:test:79979Z:priority"];
    let clear = || async {
        for key in keys {
            let _ = CLIENT.delete(key).await;
        }
        let _ = CLIENT.delete(&marker_key(keys[0])).await;
    };
    clear().await;
    for key in keys {
        CLIENT.set(key, true, 100).await.expect("寫入一般略過旗標");
    }

    let first = handle_not_found(&keys, 100).await;
    let first_ttls = [
        CLIENT.ttl(keys[0]).await.expect("ttl"),
        CLIENT.ttl(keys[1]).await.expect("ttl"),
    ];
    let second = handle_not_found(&keys, 100).await;
    let second_ttls = [
        CLIENT.ttl(keys[0]).await.expect("ttl"),
        CLIENT.ttl(keys[1]).await.expect("ttl"),
    ];
    clear().await;

    assert_eq!(first, not_found_note(false));
    assert!(
        first_ttls.iter().all(|ttl| (1..=100).contains(ttl)),
        "{first_ttls:?}"
    );
    assert_eq!(second, not_found_note(true));
    assert!(
        second_ttls.iter().all(|ttl| *ttl > 100),
        "再次 404 兩個 key 都拉長：{second_ttls:?}"
    );
    assert_eq!(handle_not_found(&[], 100).await, not_found_note(false));
}
