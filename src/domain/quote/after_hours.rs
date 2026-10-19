//! # 盤後資料筆數
//!
//! 收盤後陸續寫入、不在收盤匯總裡的資料：大盤指數（15:00 與收盤匯總並行）、三大法人與融資融券
//! （`chip_daily`，21:50）、外資持股（`qfii_history`，22:00）。完整性檢查以近期交易日的筆數判斷
//! 當天是否漏寫。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

/// 一個交易日的盤後資料筆數。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AfterHoursCounts {
    /// 交易日。
    pub date: NaiveDate,
    /// 加權指數（`index` 表 `TAIEX`）筆數，正常為 1。
    pub index: i64,
    /// 三大法人與融資融券（`chip_daily`）筆數。
    pub chip: i64,
    /// 外資持股（`qfii_history`）筆數。
    pub foreign_holding: i64,
}

/// 盤後資料的筆數查詢。
#[async_trait]
pub trait AfterHoursDataQuery: Send + Sync {
    /// `dates` 每一天的盤後資料筆數，順序與 `dates` 相同。
    async fn count_after_hours(&self, dates: &[NaiveDate]) -> Result<Vec<AfterHoursCounts>>;
}
