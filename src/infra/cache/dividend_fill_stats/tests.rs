//! 全市場填息樣本的短期快取：`/market/dividend-fill-stats`、持股的同組基準與填息通知共用的測試。

use super::*;
use crate::domain::dividend::fill_stats::YieldBucket;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(1999, 1, d).expect("日期應合法")
}

/// 命中時不再查詢；錯誤不快取，下一次會重新查詢。
#[tokio::test]
async fn caches_samples_but_not_errors() {
    let sample = FillSample {
        bucket: YieldBucket::Below3,
        trading_days_to_fill: Some(1),
        trading_days_observed: 120,
    };
    let failed =
        market_fill_samples(day(1), day(2), async { Err(anyhow::anyhow!("db down")) }).await;
    assert_eq!(failed.expect_err("應回傳錯誤").to_string(), "db down");

    let first = market_fill_samples(day(1), day(2), async { Ok(vec![sample]) })
        .await
        .expect("查詢成功");
    let cached = market_fill_samples(day(1), day(2), async {
        panic!("已快取，不應再查詢");
    })
    .await
    .expect("命中快取");
    assert_eq!((first.len(), cached.as_slice()), (1, [sample].as_slice()));
}

/// 依呼叫次序回傳預先排好的結果；用完後再呼叫視為測試錯誤。
struct ScriptedQuery(tokio::sync::Mutex<Vec<Result<Vec<FillSample>>>>);

#[async_trait::async_trait]
impl MarketFillQuery for ScriptedQuery {
    async fn fetch_market_fill_samples(
        &self,
        _since: NaiveDate,
        _until: NaiveDate,
    ) -> Result<Vec<FillSample>> {
        self.0.lock().await.remove(0)
    }
}

/// 刷新會覆蓋已快取的舊樣本；刷新失敗時保留舊樣本，之後的讀取直接命中、不再查詢。
#[tokio::test]
async fn refresh_overwrites_and_keeps_old_samples_on_failure() {
    let sample = |days| FillSample {
        bucket: YieldBucket::From3To5,
        trading_days_to_fill: Some(days),
        trading_days_observed: 120,
    };
    // 1999-02-01 往前 1 年；與其他測試的鍵不同，避免共用的全域快取互相干擾。
    let today = NaiveDate::from_ymd_opt(1999, 2, 1).expect("日期應合法");
    let query = ScriptedQuery(tokio::sync::Mutex::new(vec![
        Ok(vec![sample(5)]),
        Ok(vec![sample(7), sample(9)]),
        Err(anyhow::anyhow!("db down")),
    ]));

    let first = market_fill_samples_for(&query, today, 1)
        .await
        .expect("第一次查詢成功");
    assert_eq!(first.as_slice(), [sample(5)].as_slice());

    let refreshed = refresh_market_fill_samples_for(&query, today, 1)
        .await
        .expect("刷新成功");
    assert_eq!(refreshed, 2, "刷新應回傳新樣本數");

    let failed = refresh_market_fill_samples_for(&query, today, 1).await;
    assert_eq!(failed.expect_err("刷新失敗").to_string(), "db down");

    let cached = market_fill_samples_for(&query, today, 1)
        .await
        .expect("刷新失敗後仍命中舊快取");
    assert_eq!(
        cached.as_slice(),
        [sample(7), sample(9)].as_slice(),
        "失敗的刷新不應清掉上一次刷新的樣本"
    );
}
