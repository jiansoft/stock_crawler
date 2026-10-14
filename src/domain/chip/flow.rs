//! 三大法人期間淨買賣與連續買賣超天數（讀 `chip_daily` 的全市場資料）。
//!
//! 估計金額＝每日淨股數 × 當日收盤價加總；連續天數為資料日的買賣方向往回數，遇到方向不同或
//! 沒有買賣的一天即停，最多回看 [`STREAK_WINDOW`] 個有籌碼資料的交易日。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

/// 連續買賣超天數最多回看的交易日數，也是累計天數的上限。
pub const STREAK_WINDOW: usize = 60;

/// 合法的法人別：外資、投信、自營商與三者合計。
pub const INVESTORS: [&str; 4] = ["foreign", "trust", "dealer", "total"];

/// 一檔上市櫃股票在期間內的法人淨買賣。
#[derive(Debug, Clone, PartialEq)]
pub struct StockFlow {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 股票表市場 id（2 上市、4 上櫃）。
    pub market_id: i32,
    /// 產業名稱；未分類為空字串。
    pub industry: String,
    /// 期間累計淨股數（正為買超）。
    pub net_shares: i64,
    /// 估計淨買賣金額（元）。
    pub net_amount: f64,
    /// 期間成交股數。
    pub volume: Option<f64>,
    /// 期間成交金額（元）。
    pub trade_value: Option<f64>,
    /// 期間內淨買超的天數。
    pub buy_days: i64,
    /// 期間內淨賣超的天數。
    pub sell_days: i64,
    /// 截至資料日的連續天數：正數為連續買超、負數為連續賣超、0 為資料日沒有買賣。
    pub streak: i32,
    /// 資料日收盤價。
    pub closing_price: Option<f64>,
}

/// 一檔股票截至資料日的外資、投信連續買賣超天數（正數連續買超、負數連續賣超）。
#[derive(Debug, Clone, PartialEq)]
pub struct StockStreak {
    /// 股票代號。
    pub stock_symbol: String,
    /// 外資連續天數。
    pub foreign_streak: i16,
    /// 投信連續天數。
    pub trust_streak: i16,
}

/// 累計期間：資料日、期間第一天與實際天數。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlowPeriod {
    /// 資料日（期間最後一個交易日）。
    pub data_date: NaiveDate,
    /// 期間第一個交易日。
    pub from: NaiveDate,
    /// 實際累計的交易日數。
    pub days: usize,
}

/// 由新到舊的籌碼交易日決定累計期間與連續天數的回看起點；沒有交易日回 `None`。
pub fn flow_period(dates: &[NaiveDate], days: usize) -> Option<(FlowPeriod, NaiveDate)> {
    let data_date = *dates.first()?;
    let days = days.clamp(1, dates.len());
    let window_start = *dates.last()?;
    Some((
        FlowPeriod {
            data_date,
            from: dates[days - 1],
            days,
        },
        window_start,
    ))
}

/// 法人買賣超的查詢介面。
#[async_trait]
pub trait ChipFlowQuery: Send + Sync {
    /// `until`（含）之前最近 [`STREAK_WINDOW`] 個有籌碼資料的交易日，由新到舊。
    async fn fetch_chip_dates(&self, until: NaiveDate) -> Result<Vec<NaiveDate>>;

    /// 期間 `period` 內法人別 `investor`（[`INVESTORS`] 之一）的全體上市櫃淨買賣；
    /// 連續天數回看到 `window_start`。
    async fn fetch_flows(
        &self,
        period: FlowPeriod,
        window_start: NaiveDate,
        investor: &str,
    ) -> Result<Vec<StockFlow>>;

    /// 截至 `data_date` 各股票的外資、投信連續買賣超天數，回看到 `window_start`。
    async fn fetch_streaks(
        &self,
        data_date: NaiveDate,
        window_start: NaiveDate,
    ) -> Result<Vec<StockStreak>>;
}

/// 載入 `until`（含）之前最近 `days` 個籌碼交易日、法人別 `investor` 的全體淨買賣；
/// 沒有任何籌碼資料回 `Ok(None)`。
///
/// # Errors
///
/// 查詢失敗時回傳錯誤。
pub async fn load_flows(
    query: &impl ChipFlowQuery,
    until: NaiveDate,
    days: usize,
    investor: &str,
) -> Result<Option<(FlowPeriod, Vec<StockFlow>)>> {
    let dates = query.fetch_chip_dates(until).await?;
    let Some((period, window_start)) = flow_period(&dates, days) else {
        return Ok(None);
    };
    let rows = query.fetch_flows(period, window_start, investor).await?;
    Ok(Some((period, rows)))
}

/// 載入 `until`（含）之前最新籌碼資料日的外資、投信連續買賣超天數；沒有任何籌碼資料回 `Ok(None)`。
///
/// # Errors
///
/// 查詢失敗時回傳錯誤。
pub async fn load_streaks(
    query: &impl ChipFlowQuery,
    until: NaiveDate,
) -> Result<Option<(NaiveDate, Vec<StockStreak>)>> {
    let dates = query.fetch_chip_dates(until).await?;
    let (Some(&data_date), Some(&window_start)) = (dates.first(), dates.last()) else {
        return Ok(None);
    };
    let rows = query.fetch_streaks(data_date, window_start).await?;
    Ok(Some((data_date, rows)))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
    }

    /// 期間：資料日為第一天、天數不超過已有的交易日、回看起點為最舊一天；沒有交易日為 None。
    #[test]
    fn flow_period_counts_trading_days() {
        let dates = [day(2026, 10, 13), day(2026, 10, 12), day(2026, 10, 8)];
        let (period, window_start) = flow_period(&dates, 2).expect("有交易日");
        assert_eq!(period.data_date, day(2026, 10, 13));
        assert_eq!(period.from, day(2026, 10, 12));
        assert_eq!(period.days, 2);
        assert_eq!(window_start, day(2026, 10, 8));
        let (short, _) = flow_period(&dates, 60).expect("有交易日");
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

        async fn fetch_flows(
            &self,
            period: FlowPeriod,
            window_start: NaiveDate,
            investor: &str,
        ) -> Result<Vec<StockFlow>> {
            self.got.lock().expect("測試鎖").push(format!(
                "flows {} {} {window_start} {investor}",
                period.from, period.days
            ));
            Ok(Vec::new())
        }

        async fn fetch_streaks(
            &self,
            data_date: NaiveDate,
            window_start: NaiveDate,
        ) -> Result<Vec<StockStreak>> {
            self.got
                .lock()
                .expect("測試鎖")
                .push(format!("streaks {data_date} {window_start}"));
            Ok(Vec::new())
        }
    }

    /// 載入函式以籌碼交易日決定期間與回看起點；沒有交易日回 None 且不再查詢。
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
            [
                "flows 2026-10-12 2 2026-10-08 trust",
                "streaks 2026-10-13 2026-10-08"
            ]
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
}
