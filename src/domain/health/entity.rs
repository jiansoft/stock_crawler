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

/// 單一交易日、單一市場的本益比與股價淨值比為 0 的列數。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValuationRatioZeroCount {
    /// 交易日。
    pub date: NaiveDate,
    /// 市場別（2 上市、4 上櫃）。
    pub market_id: i32,
    /// 有成交的普通股與特別股列數（不含 ETF）。
    pub rows: i64,
    /// 本益比為 0 的列數（虧損股本來就是 0）。
    pub pe_zero: i64,
    /// 股價淨值比為 0 的列數。
    pub pb_zero: i64,
}

/// 未來報酬表中，短天期報酬缺漏的基準日。
///
/// 「長天期有值、短天期卻是空的」代表回補時結束日沒有涵蓋這個基準日之後的 5／20 個交易日；
/// 個股停牌、不常成交的債券 ETF 也會造成零星缺值，所以只列出缺漏占比達門檻的基準日。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardReturnGap {
    /// 基準日。
    pub date: NaiveDate,
    /// 較短天期為空、較長天期有值的列數。
    pub rows: i64,
}

/// 單一日期的 ETF 淨值列數（`etf_daily_nav`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EtfNavDayCount {
    /// 資料日期；台股休市時海外 ETF 仍會有當天的淨值。
    pub date: NaiveDate,
    /// 當天的列數。
    pub rows: i64,
    /// 正式單位淨值還沒回填（`nav` 為 NULL）的列數；下一個營業日的排程才會回填。
    pub missing_nav: i64,
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
    /// 檢查期間內 `chip_daily` 的連續買賣超天數還沒計算（NULL）的列數。
    pub chip_streak_missing: i64,
    /// 各衍生資料表的最新日期。
    pub derived_tables: Vec<DerivedTableLatest>,
    /// 近五年（不含今年）發放、除權息日仍是「尚未公布」的股利列數。
    pub stale_dividend_placeholders: i64,
    /// 年度合計列與各期明細加總（現金或股票股利）不符的組數。
    pub dividend_total_mismatches: i64,
    /// 月營收的最新年月（`YYYYMM`）；沒有資料時為 `None`。
    pub latest_revenue_month: Option<i64>,
    /// 集保千張大戶週摘要（`holder_distribution`）的最新資料日；沒有資料時為 `None`。
    pub latest_holder_week: Option<NaiveDate>,
    /// 集保全部持股分級（`holder_distribution_level`）的最新資料日；沒有資料時為 `None`。
    pub latest_holder_level_week: Option<NaiveDate>,
    /// `holder_distribution_level` 最新一週的列數（每檔 17 列：15 級、差異數調整與合計）。
    pub holder_level_rows: i64,
    /// `holder_distribution` 最新一週 400 張以上人數（`holders_400`）是空值的檔數。
    pub holders_400_missing: i64,
    /// 董監持股與設質（`insider_holding`）的最新月份（該月 1 日）；沒有資料時為 `None`。
    pub latest_insider_month: Option<NaiveDate>,
    /// 檢查期間內每個交易日、每個市場的本益比與股價淨值比為 0 的列數。
    pub valuation_ratio_zeros: Vec<ValuationRatioZeroCount>,
    /// 未來報酬表（整張表，從最早有 5 日報酬的基準日起）短天期缺漏的基準日，由舊到新。
    pub forward_return_gaps: Vec<ForwardReturnGap>,
    /// 期間內（含基準期）每天的 ETF 淨值列數，由舊到新。
    pub etf_nav_counts: Vec<EtfNavDayCount>,
    /// 基本資料（`etf_profile`）在期限前就沒再更新、或上市已久仍沒有基本資料的 ETF 代號。
    pub stale_etf_profiles: Vec<String>,
}
