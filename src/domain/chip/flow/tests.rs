//! 三大法人期間淨買賣與連續買賣超天數（讀 `chip_daily` 的全市場資料）的測試。

use std::sync::Mutex;

use super::*;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
}

/// 期間：資料日為第一天、天數不超過已有的交易日；沒有交易日為 None。
#[test]
fn flow_period_counts_trading_days() {
    let dates = [day(2026, 10, 13), day(2026, 10, 12), day(2026, 10, 8)];
    let period = flow_period(&dates, 2).expect("有交易日");
    assert_eq!(period.data_date, day(2026, 10, 13));
    assert_eq!(period.from, day(2026, 10, 12));
    assert_eq!(period.days, 2);
    let short = flow_period(&dates, 60).expect("有交易日");
    assert_eq!((short.from, short.days), (day(2026, 10, 8), 3));
    assert!(flow_period(&[], 1).is_none());
}

/// 記錄查詢條件的假查詢。
#[derive(Default)]
struct FakeChip {
    dates: Vec<NaiveDate>,
    got: Mutex<Vec<String>>,
}

#[async_trait]
impl ChipFlowQuery for FakeChip {
    async fn fetch_chip_dates(&self, _until: NaiveDate) -> Result<Vec<NaiveDate>> {
        Ok(self.dates.clone())
    }

    async fn fetch_flows(&self, period: FlowPeriod, investor: &str) -> Result<Vec<StockFlow>> {
        self.got
            .lock()
            .expect("測試鎖")
            .push(format!("flows {} {} {investor}", period.from, period.days));
        Ok(Vec::new())
    }

    async fn fetch_streaks(&self, data_date: NaiveDate) -> Result<Vec<StockStreak>> {
        self.got
            .lock()
            .expect("測試鎖")
            .push(format!("streaks {data_date}"));
        Ok(Vec::new())
    }
}

/// 載入函式以籌碼交易日決定期間與資料日；沒有交易日回 None 且不再查詢。
#[tokio::test]
async fn load_functions_use_chip_dates() {
    let chip = FakeChip {
        dates: vec![day(2026, 10, 13), day(2026, 10, 12), day(2026, 10, 8)],
        ..Default::default()
    };
    let until = day(2026, 10, 14);
    let (period, rows) = load_flows(&chip, until, 2, "trust")
        .await
        .expect("成功")
        .expect("有資料");
    assert_eq!(period.from, day(2026, 10, 12));
    assert!(rows.is_empty());
    let (data_date, _) = load_streaks(&chip, until)
        .await
        .expect("成功")
        .expect("有資料");
    assert_eq!(data_date, day(2026, 10, 13));
    assert_eq!(
        chip.got.lock().expect("測試鎖").clone(),
        ["flows 2026-10-12 2 trust", "streaks 2026-10-13"]
    );

    let empty = FakeChip::default();
    assert!(
        load_flows(&empty, until, 1, "foreign")
            .await
            .expect("成功")
            .is_none()
    );
    assert!(load_streaks(&empty, until).await.expect("成功").is_none());
    assert!(empty.got.lock().expect("測試鎖").is_empty());
}
