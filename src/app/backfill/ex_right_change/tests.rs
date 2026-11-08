//! 除權息日漲跌修正的測試。

use std::sync::Mutex;

use anyhow::anyhow;
use async_trait::async_trait;
use rust_decimal_macros::dec;

use super::*;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("有效日期")
}

/// 記錄寫入參數的假倉儲；每筆基準價算更新一列。
#[derive(Default)]
struct FakeRepo {
    calls: Mutex<Vec<(Vec<String>, bool)>>,
    fail: bool,
}

#[async_trait]
impl ExRightChangeRepository for FakeRepo {
    async fn apply_ex_right_references(
        &self,
        references: &[ExRightReference],
        with_stats: bool,
    ) -> Result<u64> {
        if self.fail {
            return Err(anyhow!("寫入失敗"));
        }
        self.calls.lock().expect("測試鎖").push((
            references.iter().map(|r| r.stock_symbol.clone()).collect(),
            with_stats,
        ));
        Ok(references.len() as u64)
    }
}

fn reference(symbol: &str, date: NaiveDate) -> ExRightReference {
    ExRightReference {
        stock_symbol: symbol.to_owned(),
        date,
        reference_price: dec!(10),
    }
}

/// 跨年的期間切成每年一段，頭尾依實際起訖日。
#[test]
fn yearly_windows_split_by_calendar_year() {
    assert_eq!(
        yearly_windows(day(2023, 6, 1), day(2025, 2, 3)),
        [
            (day(2023, 6, 1), day(2023, 12, 31)),
            (day(2024, 1, 1), day(2024, 12, 31)),
            (day(2025, 1, 1), day(2025, 2, 3)),
        ]
    );
    assert_eq!(
        yearly_windows(day(2026, 10, 13), day(2026, 10, 13)),
        [(day(2026, 10, 13), day(2026, 10, 13))]
    );
    assert!(yearly_windows(day(2026, 10, 14), day(2026, 10, 13)).is_empty());
}

/// 每段期間依序抓上市、上櫃並寫入；第一次之後才停頓；一個市場失敗只記錄。
#[tokio::test]
async fn run_fetches_both_markets_and_skips_failures() {
    let repo = FakeRepo::default();
    let pauses = Mutex::new(0);
    let windows = [
        (day(2024, 1, 1), day(2024, 12, 31)),
        (day(2025, 1, 1), day(2025, 3, 1)),
    ];
    let summary = run(
        &repo,
        &windows,
        true,
        |market, from, _to| async move {
            match (market, from.year()) {
                (Market::Otc, 2024) => Err(anyhow!("逾時")),
                (Market::Listed, year) => Ok(vec![reference(&format!("L{year}"), from)]),
                (Market::Otc, year) => Ok(vec![
                    reference(&format!("O{year}"), from),
                    reference(&format!("P{year}"), from),
                ]),
            }
        },
        || {
            *pauses.lock().expect("測試鎖") += 1;
            async {}
        },
    )
    .await
    .expect("部分失敗仍成功");

    assert_eq!(
        summary,
        Summary {
            requests: 4,
            failed: 1,
            fetched: 4,
            updated: 4,
        }
    );
    assert_eq!(*pauses.lock().expect("測試鎖"), 3);
    let calls = repo.calls.lock().expect("測試鎖");
    let symbols: Vec<Vec<String>> = calls.iter().map(|(s, _)| s.clone()).collect();
    assert_eq!(
        symbols,
        [
            vec!["L2024".to_owned()],
            vec!["L2025".to_owned()],
            vec!["O2025".to_owned(), "P2025".to_owned()],
        ]
    );
    assert!(calls.iter().all(|(_, with_stats)| *with_stats));
}

/// 全部抓取失敗回錯誤；寫入失敗直接回錯誤。
#[tokio::test]
async fn run_fails_when_all_fetches_fail_or_write_fails() {
    let windows = [(day(2026, 10, 13), day(2026, 10, 13))];
    let error = run(
        &FakeRepo::default(),
        &windows,
        false,
        |_, _, _| async { Err(anyhow!("逾時")) },
        || async {},
    )
    .await
    .expect_err("全部失敗");
    assert!(
        error.to_string().contains("全部抓取失敗（2 次）"),
        "{error}"
    );

    let failing = FakeRepo {
        fail: true,
        ..Default::default()
    };
    let error = run(
        &failing,
        &windows,
        false,
        |_, from, _| async move { Ok(vec![reference("2330", from)]) },
        || async {},
    )
    .await
    .expect_err("寫入失敗");
    assert!(error.to_string().contains("寫入失敗"), "{error}");
}
