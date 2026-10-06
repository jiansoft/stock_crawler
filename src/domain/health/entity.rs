//! 資料健康檢查的量測值。
//!
//! 只放「從資料庫量到的數字」，判斷是否異常的門檻在 app 層
//! （`app::event::taiwan_stock::data_health`），方便單元測試。

use chrono::NaiveDate;

use crate::domain::quote::entity::MarketTradedCount;

/// 衍生資料表的最新日期。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedTableLatest {
    /// 資料表名稱，例如 `estimate`。
    pub table: &'static str,
    /// 資料表內最新的日期；表內沒有資料時為 `None`。
    pub latest: Option<NaiveDate>,
}

/// 一次健康檢查量到的全部數字。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataHealthSnapshot {
    /// 期間內（含基準期）每個交易日上市、上櫃有成交的檔數。
    pub market_counts: Vec<MarketTradedCount>,
    /// 檢查期間內有任何日報價的日期。
    pub quoted_dates: Vec<NaiveDate>,
    /// 檢查期間內有收盤價、前 400 天內已有至少 5 筆報價，5 日均線卻是 0 的列數。
    pub missing_moving_averages: i64,
    /// 檢查期間內漲跌幅與「漲跌 ÷ 參考價」不符（差超過 0.01 個百分點）的列數。
    pub change_range_mismatches: i64,
    /// 檢查期間內 `year`／`month`／`day` 與日期不符的列數。
    pub misdated_rows: i64,
    /// 各衍生資料表的最新日期。
    pub derived_tables: Vec<DerivedTableLatest>,
    /// 發放年度早於今年、除權息日仍是「尚未公布」的股利列數。
    pub stale_dividend_placeholders: i64,
    /// 年度合計列與各期明細加總（現金或股票股利）不符的組數。
    pub dividend_total_mismatches: i64,
    /// 月營收的最新年月（`YYYYMM`）；沒有資料時為 `None`。
    pub latest_revenue_month: Option<i64>,
}
