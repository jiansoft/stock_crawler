use crate::domain::dividend::entity::Dividend;
use crate::infra::database::table::dividend::extension::stock_dividend_info::StockDividendInfo;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Local, NaiveDate};

use crate::infra::database::table::dividend::extension::stock_dividend_payable_date_info::StockDividendPayableDateInfo;

/// 股利領域之倉儲介面 (Repository Trait)。
///
/// 定義對 `Dividend` 聚合根之讀取與持久化合約，將資料庫細節與領域層隔離。
#[async_trait]
pub trait DividendRepository: Send + Sync {
    /// 依證券代號查詢該證券的所有股利年度。
    async fn fetch_years_by_security_code(&self, security_code: &str) -> Result<Vec<i32>>;

    /// 依代號、年份及持有（建立）時間，查詢所有可能重疊的股利發放資料。
    async fn fetch_dividends_summary_by_date(
        &self,
        security_code: &str,
        year: i32,
        created_time: DateTime<Local>,
    ) -> Result<Vec<Dividend>>;

    /// 取得所有尚未計算或更新配息率（`payout_ratio` 為 0 且盈餘配發不為 0）的股利資料。
    async fn fetch_without_payout_ratio(&self) -> Result<Vec<Dividend>>;

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
