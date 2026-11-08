//! 官方本益比與股價淨值比回補的測試。

use std::sync::Mutex;

use anyhow::anyhow;
use async_trait::async_trait;
use rust_decimal_macros::dec;

use super::*;

/// 依市場回傳固定缺值日期、記下寫入內容的假倉儲。
#[derive(Default)]
struct FakeRepo {
    listed: Vec<NaiveDate>,
    otc: Vec<NaiveDate>,
    filled: Mutex<Vec<(NaiveDate, usize)>>,
    synced: Mutex<usize>,
}

#[async_trait]
impl OfficialRatioRepository for FakeRepo {
    async fn fetch_dates_to_update(
        &self,
        market_id: i32,
        _from: NaiveDate,
        _to: NaiveDate,
        policy: PePolicy,
    ) -> Result<Vec<NaiveDate>> {
        assert_eq!(
            policy,
            if market_id == 2 {
                PePolicy::FillZeroOnly
            } else {
                PePolicy::Authoritative
            }
        );
        Ok(if market_id == 2 {
            self.listed.clone()
        } else {
            self.otc.clone()
        })
    }

    async fn fill_official_ratios(
        &self,
        date: NaiveDate,
        ratios: &[OfficialRatio],
        _policy: PePolicy,
    ) -> Result<u64> {
        self.filled.lock().unwrap().push((date, ratios.len()));
        Ok(ratios.len() as u64)
    }

    async fn sync_latest_quotes(&self) -> Result<u64> {
        *self.synced.lock().unwrap() += 1;
        Ok(3)
    }
}

fn day(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2020, 6, day).expect("有效日期")
}

fn ratio(symbol: &str) -> OfficialRatio {
    OfficialRatio {
        stock_symbol: symbol.to_owned(),
        price_earning_ratio: Some(dec!(10)),
        price_to_book_ratio: None,
    }
}

/// 兩市場各自的缺值日都抓一次；單日失敗不中斷；請求之間才停頓。
#[tokio::test]
async fn run_visits_missing_dates_and_tolerates_failures() {
    let repo = FakeRepo {
        listed: vec![day(29), day(30)],
        otc: vec![day(30)],
        ..Default::default()
    };
    let pauses = Mutex::new(0);
    let summary = run(
        &repo,
        day(1),
        day(30),
        |market, date| async move {
            match (market, date == day(29)) {
                (Market::Listed, true) => Err(anyhow!("逾時")),
                (Market::Listed, false) => Ok(vec![ratio("1101"), ratio("2330")]),
                (Market::Otc, _) => Ok(vec![ratio("6488")]),
            }
        },
        || {
            *pauses.lock().unwrap() += 1;
            async {}
        },
    )
    .await
    .expect("部分失敗不是錯誤");
    assert_eq!(
        summary,
        Summary {
            requests: 3,
            failed: 1,
            fetched_rows: 3,
            updated_rows: 3,
            synced_latest_rows: 3
        }
    );
    assert_eq!(
        *repo.synced.lock().unwrap(),
        1,
        "有更新才同步最新報價，且只同步一次"
    );
    assert_eq!(*pauses.lock().unwrap(), 2);
    assert_eq!(
        *repo.filled.lock().unwrap(),
        vec![(day(30), 2), (day(30), 1)]
    );
}

/// 全部失敗回錯誤；沒有缺值日不發請求、也不算失敗。
#[tokio::test]
async fn run_fails_only_when_every_request_fails() {
    let repo = FakeRepo {
        listed: vec![day(30)],
        ..Default::default()
    };
    let result = run(
        &repo,
        day(1),
        day(30),
        |_, _| async { Err(anyhow!("封鎖")) },
        || async {},
    )
    .await;
    assert!(result.is_err());

    let idle_repo = FakeRepo::default();
    let idle = run(
        &idle_repo,
        day(1),
        day(30),
        |_, _| async { Ok(Vec::new()) },
        || async {},
    )
    .await
    .expect("沒有缺值不是錯誤");
    assert_eq!(idle, Summary::default());
    assert_eq!(*idle_repo.synced.lock().unwrap(), 0, "沒有更新不同步");
    assert_eq!(
        (Market::Listed.market_id(), Market::Otc.market_id()),
        (2, 4)
    );
}

/// 排程回看區間含今天共 7 天。
#[test]
fn recent_window_covers_seven_days() {
    let today = NaiveDate::from_ymd_opt(2026, 10, 12).expect("有效日期");
    assert_eq!(
        recent_window(today),
        (
            NaiveDate::from_ymd_opt(2026, 10, 6).expect("有效日期"),
            today
        )
    );
}
