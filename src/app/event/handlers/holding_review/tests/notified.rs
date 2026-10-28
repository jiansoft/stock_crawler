//! 已通知持股的記錄與比對（Redis）測試。

use super::*;

#[test]
fn parse_notified_symbols_ignores_blank_entries() {
    let symbols = EventDispatcher::parse_notified_symbols("2330, 2454,,");

    assert_eq!(
        symbols,
        BTreeSet::from(["2330".to_string(), "2454".to_string()])
    );
    assert!(EventDispatcher::parse_notified_symbols("").is_empty());
}

#[test]
fn find_new_symbols_excludes_already_notified() {
    let notified = BTreeSet::from(["2330".to_string()]);

    let new_symbols = EventDispatcher::find_new_symbols(&notified, ["2330", "2454"]);

    assert_eq!(new_symbols, BTreeSet::from(["2454".to_string()]));
    assert!(EventDispatcher::find_new_symbols(&notified, ["2330"]).is_empty());
}

/// 已通知集合存在 Redis：第一次全部是新的、不逐檔標記；之後只標記新增代號；沒有新增就不送出。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn send_if_new_holdings_tracks_notified_symbols_in_redis() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 send_if_new_holdings_tracks_notified_symbols_in_redis：無 Redis 連接");
        return;
    }
    let key = "test:holding_review:notified:79979";
    let _ = CLIENT.delete(key).await;

    let mut highlights = Vec::new();
    let first = EventDispatcher::send_if_new_holdings(key, 60, ["79979A", "79979B"], |new| {
        highlights.push(new.cloned());
        "第一次".to_owned()
    })
    .await;
    let repeated = EventDispatcher::send_if_new_holdings(key, 60, ["79979B", "79979A"], |new| {
        highlights.push(new.cloned());
        "重複".to_owned()
    })
    .await;
    let added = EventDispatcher::send_if_new_holdings(key, 60, ["79979A", "79979C"], |new| {
        highlights.push(new.cloned());
        "補發".to_owned()
    })
    .await;
    let stored = EventDispatcher::load_notified_symbols(key).await;
    let _ = CLIENT.delete(key).await;

    assert!(first && !repeated && added);
    assert_eq!(
        highlights,
        [None, Some(BTreeSet::from(["79979C".to_owned()]))]
    );
    assert_eq!(
        stored,
        BTreeSet::from([
            "79979A".to_owned(),
            "79979B".to_owned(),
            "79979C".to_owned()
        ])
    );
}

/// 外資持股去重鍵：寫入後視為已通知，刪除後恢復為未通知。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn is_already_notified_reads_redis_key() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 is_already_notified_reads_redis_key：無 Redis 連接");
        return;
    }
    let key = "test:holding_review:qfii:79979";
    let _ = CLIENT.delete(key).await;
    assert!(!EventDispatcher::is_already_notified(key).await);
    CLIENT.set(key, "2026-10-13", 60).await.expect("寫入去重鍵");
    assert!(EventDispatcher::is_already_notified(key).await);
    let _ = CLIENT.delete(key).await;
    assert!(!EventDispatcher::is_already_notified(key).await);
}
