use crate::domain::dividend::entity::{
    Dividend, PayoutRatioCandidate, PayoutRatios, StockDividendInfo, StockDividendPayableDateInfo,
};
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Local, NaiveDate};

/// 股利領域之倉儲介面 (Repository Trait)。
///
/// 定義對 `Dividend` 聚合根之讀取與持久化合約，將資料庫細節與領域層隔離。
#[async_trait]
pub trait DividendRepository: Send + Sync {
    /// 依證券代號查詢該證券的所有股利年度。
    async fn fetch_years_by_security_code(&self, security_code: &str) -> Result<Vec<i32>>;

    /// 取得仍在上市櫃的股利採集候選；已有年配資料仍須定期檢查新增配息。
    async fn fetch_dividend_refresh_candidates(&self) -> Result<Vec<String>>;

    /// 取得指定年度與多次配息相關的股利資料。
    async fn fetch_multiple_dividends_for_year(&self, year: i32) -> Result<Vec<Dividend>>;

    /// 取得指定發放年度的所有股利資料。
    ///
    /// 供除權息公告掃描一次載入整年資料後在記憶體比對，
    /// 避免對數百檔股票各發一次查詢。
    async fn fetch_by_years(&self, years: &[i32]) -> Result<Vec<Dividend>>;

    /// 合併並更新指定股票在指定發放年度的年度股利合計。
    async fn upsert_annual_total_dividend(&self, security_code: &str, year: i32) -> Result<()>;

    /// 找出帶有配息日期的年度合計列，回傳 (證券代號, 發放年度)。
    ///
    /// 合計列的日期應該一律是 `'-'`；帶有真實日期代表它是由既有的年度配息列原地 upsert
    /// 而成（股票從年配改成分期配發時兩者主鍵相同），這種列會被下游誤認成一次真實配息。
    async fn fetch_stale_annual_total_dividends(&self) -> Result<Vec<(String, i32)>>;

    /// 取得指定年度尚未有配息日或發放日的股息數據。
    async fn fetch_unpublished_dividend_date_or_payable_date_for_specified_year(
        &self,
        year: i32,
    ) -> Result<Vec<Dividend>>;

    /// 更新股利發放日期相關資訊（除息日、除權日、發放日）。
    async fn update_dividend_date(&self, dividend: &Dividend) -> Result<()>;

    /// 依代號、年份及持有（建立）時間，查詢所有可能重疊的股利發放資料。
    async fn fetch_dividends_summary_by_date(
        &self,
        security_code: &str,
        year: i32,
        created_time: DateTime<Local>,
    ) -> Result<Vec<Dividend>>;

    /// 取得待計算盈餘分配率的股利列，並帶出同期間的每股盈餘。
    async fn fetch_payout_ratio_candidates(&self) -> Result<Vec<PayoutRatioCandidate>>;

    /// 批次寫回計算好的盈餘分配率，回傳實際更新的列數。
    async fn update_payout_ratios(&self, ratios: &[PayoutRatios]) -> Result<u64>;

    /// 取得指定日期有除權或除息事件的股票資料與參考收盤價。
    async fn fetch_stocks_with_dividends_on_date(
        &self,
        date: NaiveDate,
    ) -> Result<Vec<StockDividendInfo>>;

    /// 取得指定日期為股利發放日的股票與配發資訊。
    async fn fetch_payable_date_info_on_date(
        &self,
        date: NaiveDate,
    ) -> Result<Vec<StockDividendPayableDateInfo>>;

    /// 儲存或更新單筆股利實體。
    async fn save(&self, dividend: &Dividend) -> Result<()>;
}
