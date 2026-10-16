//! 基於 PostgreSQL 的股利倉儲：trait 方法委派給各子模組。
//!
//! 子模組：[`query`]（讀取查詢與配息日期更新）、[`annual_total`]（年度合計列）、[`payout`]
//! （盈餘分配率）、[`save`]（儲存與所屬年度更正）、[`mapping`]（資料列轉換）。

mod annual_total;
mod mapping;
mod payout;
mod query;
mod save;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Local, NaiveDate};

use crate::domain::dividend::entity::{
    Dividend, StockDividendInfo as DomainStockDividendInfo,
    StockDividendPayableDateInfo as DomainStockDividendPayableDateInfo,
};
use crate::domain::dividend::payout::{PayoutDividend, PayoutRatios, PeriodEarnings};
use crate::domain::dividend::repository::DividendRepository;
use crate::infra::database::table::dividend::extension::{
    stock_dividend_info, stock_dividend_payable_date_info,
};

/// 基於 PostgreSQL 的股利倉儲實現 (PgDividendRepository)。
pub struct PgDividendRepository;

impl PgDividendRepository {
    /// 建立新的 PgDividendRepository 實例。
    pub fn new() -> Self {
        PgDividendRepository
    }
}

impl Default for PgDividendRepository {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DividendRepository for PgDividendRepository {
    /// 依證券代號查詢該證券的所有股利年度（見 [`query::fetch_years_by_security_code`]）。
    async fn fetch_years_by_security_code(&self, security_code: &str) -> Result<Vec<i32>> {
        query::fetch_years_by_security_code(security_code).await
    }

    /// 取得仍在上市櫃的採集候選（見 [`query::fetch_dividend_refresh_candidates`]）。
    async fn fetch_dividend_refresh_candidates(&self) -> Result<Vec<String>> {
        query::fetch_dividend_refresh_candidates().await
    }

    /// 取得指定年度與多次配息相關的股利資料。
    async fn fetch_multiple_dividends_for_year(&self, year: i32) -> Result<Vec<Dividend>> {
        query::fetch_multiple_dividends_for_year(year).await
    }

    /// 取得指定發放年度的所有股利資料。
    async fn fetch_by_years(&self, years: &[i32]) -> Result<Vec<Dividend>> {
        query::fetch_by_years(years).await
    }

    /// 合併並更新指定股票在指定發放年度的年度股利合計（見
    /// [`annual_total::upsert_annual_total_dividend`]）。
    async fn upsert_annual_total_dividend(&self, security_code: &str, year: i32) -> Result<()> {
        annual_total::upsert_annual_total_dividend(security_code, year).await
    }

    /// 找出帶有配息日期的年度合計列，回傳 (證券代號, 發放年度)。
    async fn fetch_stale_annual_total_dividends(&self) -> Result<Vec<(String, i32)>> {
        annual_total::fetch_stale_annual_total_dividends().await
    }

    /// 取得指定年度尚未有配息日或發放日的股息數據（見
    /// [`query::fetch_unpublished_dividend_date_or_payable_date_for_specified_year`]）。
    async fn fetch_unpublished_dividend_date_or_payable_date_for_specified_year(
        &self,
        year: i32,
    ) -> Result<Vec<Dividend>> {
        query::fetch_unpublished_dividend_date_or_payable_date_for_specified_year(year).await
    }

    /// 更新股利發放日期相關資訊（除息日、除權日、發放日）。
    async fn update_dividend_date(&self, dividend: &Dividend) -> Result<()> {
        query::update_dividend_date(dividend).await
    }

    /// 依代號、年份及持有（建立）時間，查詢所有可能重疊的股利發放資料（見
    /// [`query::fetch_dividends_summary_by_date`]）。
    async fn fetch_dividends_summary_by_date(
        &self,
        security_code: &str,
        year: i32,
        created_time: DateTime<Local>,
    ) -> Result<Vec<Dividend>> {
        query::fetch_dividends_summary_by_date(security_code, year, created_time).await
    }

    /// 取得計算盈餘分配率所需的股利列與各期財報每股盈餘（見 [`payout::fetch_payout_ratio_inputs`]）。
    async fn fetch_payout_ratio_inputs(
        &self,
    ) -> Result<(Vec<PayoutDividend>, Vec<PeriodEarnings>)> {
        payout::fetch_payout_ratio_inputs().await
    }

    /// 批次寫回計算好的盈餘分配率、分母 EPS 與涵蓋期間，回傳實際更新的列數。
    async fn update_payout_ratios(&self, ratios: &[PayoutRatios]) -> Result<u64> {
        payout::update_payout_ratios(ratios).await
    }

    /// 取得指定日期有除權或除息事件的股票資料與參考收盤價。
    async fn fetch_stocks_with_dividends_on_date(
        &self,
        date: NaiveDate,
    ) -> Result<Vec<DomainStockDividendInfo>> {
        let table_list = stock_dividend_info::fetch_stocks_with_dividends_on_date(date).await?;
        Ok(table_list
            .into_iter()
            .map(DomainStockDividendInfo::from)
            .collect())
    }

    async fn fetch_payable_date_info_on_date(
        &self,
        date: NaiveDate,
    ) -> Result<Vec<DomainStockDividendPayableDateInfo>> {
        let table_list = stock_dividend_payable_date_info::fetch(date).await?;
        Ok(table_list
            .into_iter()
            .map(DomainStockDividendPayableDateInfo::from)
            .collect())
    }

    /// 儲存或更新股利（見 [`save::save`]）。
    async fn save(&self, dividend: &Dividend) -> Result<()> {
        save::save(dividend).await
    }
}

#[cfg(test)]
mod tests;
