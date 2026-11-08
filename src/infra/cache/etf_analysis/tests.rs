//! ETF 比較與持股穿透共用的短期查詢快取；個人持股與市值每次重新讀取的測試。

use super::*;
#[tokio::test]
async fn coalesces_misses_and_invalidates_without_caching_errors() {
    use std::sync::atomic::AtomicUsize;
    let cache = EtfAnalysisCache::default();
    let calls = AtomicUsize::new(0);
    let load = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
        Ok(None)
    };
    let (a, b) = tokio::join!(
        cache.detail("0050", 1, load()),
        cache.detail("0050", 1, load())
    );
    assert!(a.unwrap().is_none() && b.unwrap().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    cache.invalidate();
    assert!(
        cache
            .detail("0050", 1, async { Err(anyhow::anyhow!("失敗")) })
            .await
            .is_err()
    );
    cache.detail("0050", 1, load()).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn does_not_cache_missing_etf_and_separates_nav_days() {
    use std::sync::atomic::AtomicUsize;
    let cache = EtfAnalysisCache::default();
    let calls = AtomicUsize::new(0);
    let missing = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    };
    assert!(cache.detail("00999", 1, missing()).await.unwrap().is_none());
    assert!(cache.detail("00999", 1, missing()).await.unwrap().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 2, "查無資料不可快取");

    let found = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(Some(EtfDetail::default()))
    };
    cache.detail("0050", 1, found()).await.unwrap();
    cache.detail("0050", 1, found()).await.unwrap();
    cache.detail("0050", 20, found()).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 4, "nav_days 不同要各自載入");
}

#[tokio::test]
async fn shared_error_keeps_the_cause_chain() {
    let cache = EtfAnalysisCache::default();
    let error = cache
        .detail("0050", 1, async {
            Err(anyhow::anyhow!("底層錯誤").context("查詢失敗"))
        })
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "查詢失敗");
    assert_eq!(format!("{error:#}"), "查詢失敗: 底層錯誤");
}
#[tokio::test]
async fn write_then_invalidate_reads_fresh_value_immediately() {
    use std::sync::atomic::AtomicU32;
    let cache = EtfAnalysisCache::default();
    let version = Arc::new(AtomicU32::new(1));

    let v_clone = Arc::clone(&version);
    let load = move || {
        let v = Arc::clone(&v_clone);
        async move {
            let cur = v.load(Ordering::SeqCst);
            let mut d = EtfDetail::default();
            d.overview.name = format!("版本_{cur}");
            Ok(Some(d))
        }
    };

    // 第一次讀取：版本 1
    let first = cache.detail("0050", 1, load()).await.unwrap().unwrap();
    assert_eq!(first.overview.name, "版本_1");

    // 未失效前再次讀取：命中快取，仍為版本 1
    version.store(2, Ordering::SeqCst);
    let cached = cache.detail("0050", 1, load()).await.unwrap().unwrap();
    assert_eq!(cached.overview.name, "版本_1");

    // 寫入/回補完成觸發失效（世代推進）
    cache.invalidate();

    // 立即讀取新值：應獲得版本 2，舊快取徹底作廢
    let fresh = cache.detail("0050", 1, load()).await.unwrap().unwrap();
    assert_eq!(fresh.overview.name, "版本_2");
}
