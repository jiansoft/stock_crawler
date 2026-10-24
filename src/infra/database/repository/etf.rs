//! ETF 專區的 PostgreSQL 倉儲實作。
//!
//! 對應 `etf_daily_nav`、`etf_profile`、`etf_holding`、`etf_allocation`、`etf_performance`、
//! `etf_snapshot`（見 `etc/sql/etf.sql`）。trait 方法委派給各子模組：[`nav`]（每日淨值、回補與
//! 申贖）、[`profile`]（清單與基本資料）、[`holding`]（前十大持股、配置與每期歷史）、
//! [`snapshot`]（含息績效與清單頁快照）。

mod holding;
mod nav;
mod profile;
mod snapshot;
pub(crate) mod tracking;

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::{
    domain::etf::{
        EtfHoldingPeriods, EtfNavHistoryPoint, EtfNavRecord, EtfPerformance, EtfPortfolio,
        EtfProfileRecord, EtfRepository, EtfWeeklyFlow, PremiumPoint, UnitsChangePoint,
    },
    domain::quote::technical::PriceBar,
};

/// 基於 PostgreSQL 的 ETF 倉儲。
#[derive(Debug, Clone, Copy, Default)]
pub struct PgEtfRepository;

impl PgEtfRepository {
    /// 建立實例。
    pub fn new() -> Self {
        Self
    }

    /// 加權指數已留存的市場交易日，由新到舊，供連續折溢價判定。
    pub(crate) async fn recent_trading_dates(
        &self,
        until: NaiveDate,
        limit: i64,
    ) -> Result<Vec<NaiveDate>> {
        Ok(sqlx::query_scalar("SELECT date FROM \"index\" WHERE category = 'TAIEX' AND date <= $1 ORDER BY date DESC LIMIT $2")
            .bind(until).bind(limit).fetch_all(crate::infra::database::get_connection()).await?)
    }
}

#[async_trait]
impl EtfRepository for PgEtfRepository {
    async fn listed_etfs(&self) -> Result<Vec<(String, String)>> {
        profile::listed_etfs().await
    }

    async fn save_navs(&self, records: &[EtfNavRecord]) -> Result<u64> {
        nav::save_navs(records).await
    }

    async fn insert_nav_history(
        &self,
        stock_symbol: &str,
        points: &[EtfNavHistoryPoint],
    ) -> Result<u64> {
        nav::insert_nav_history(stock_symbol, points).await
    }

    async fn save_profiles(&self, profiles: &[EtfProfileRecord]) -> Result<u64> {
        profile::save_profiles(profiles).await
    }

    async fn insert_missing_profiles(&self, profiles: &[EtfProfileRecord]) -> Result<u64> {
        profile::insert_missing_profiles(profiles).await
    }

    async fn replace_portfolio(&self, portfolio: &EtfPortfolio) -> Result<u64> {
        holding::replace_portfolio(portfolio).await
    }

    async fn nav_series(
        &self,
        stock_symbols: &[String],
        from: NaiveDate,
    ) -> Result<HashMap<String, Vec<PriceBar>>> {
        nav::nav_series(stock_symbols, from).await
    }

    async fn save_performances(&self, performances: &[EtfPerformance]) -> Result<u64> {
        snapshot::save_performances(performances).await
    }

    async fn refresh_snapshot(&self) -> Result<u64> {
        snapshot::refresh_snapshot().await
    }

    async fn holding_periods(
        &self,
        stock_symbols: &[String],
        recorded_since: NaiveDate,
    ) -> Result<Vec<EtfHoldingPeriods>> {
        holding::holding_periods(stock_symbols, recorded_since).await
    }

    async fn weekly_flows(&self, days: i64) -> Result<Vec<EtfWeeklyFlow>> {
        nav::weekly_flows(days).await
    }

    async fn latest_units(
        &self,
        stock_symbols: &[String],
    ) -> Result<HashMap<String, UnitsChangePoint>> {
        nav::latest_units(stock_symbols).await
    }

    async fn recent_premiums(
        &self,
        stock_symbols: &[String],
        days: i64,
    ) -> Result<HashMap<String, Vec<PremiumPoint>>> {
        nav::recent_premiums(stock_symbols, days).await
    }
}

#[cfg(test)]
mod tests;
