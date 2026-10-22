use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use crate::domain::quote::technical::PriceBar;

use super::{
    EtfHoldingPeriods, EtfNavHistoryPoint, EtfNavRecord, EtfPerformance, EtfPortfolio,
    EtfProfileRecord, EtfWeeklyFlow, PremiumPoint, UnitsChangePoint,
};

/// ETF 專區資料的倉儲合約。寫入方法回傳實際新增或變更的列數。
#[async_trait]
pub trait EtfRepository: Send + Sync {
    /// 目前上市櫃中的 ETF（產業 9001、未下市）代號與名稱，依代號排序。
    async fn listed_etfs(&self) -> Result<Vec<(String, String)>>;

    /// 寫入（或更正）淨值資料，並以每筆的「前一營業日單位淨值」回填該 ETF 前一個交易日的正式淨值；
    /// 那天沒有資料列時補一列只有正式淨值的資料。
    async fn save_navs(&self, records: &[EtfNavRecord]) -> Result<u64>;

    /// 回補一檔 ETF 的歷史正式淨值與市價，折溢價由兩者算出；已發行單位留空。
    /// 新增還沒有的日期；已有的列只補空欄（正式淨值、沒有單位數的列的市價與折溢價），MIS 寫入的值不動。
    async fn insert_nav_history(
        &self,
        stock_symbol: &str,
        points: &[EtfNavHistoryPoint],
    ) -> Result<u64>;

    /// 寫入（或更正）基本資料；不動 `manual_override`。
    async fn save_profiles(&self, profiles: &[EtfProfileRecord]) -> Result<u64>;

    /// 為還沒有基本資料的 ETF 補上推斷的分類；先前推斷的列跟著最新規則更新，來源（Yahoo）寫入的列不動。
    async fn insert_missing_profiles(&self, profiles: &[EtfProfileRecord]) -> Result<u64>;

    /// 以最新一期取代一檔 ETF 的前十大持股、行業比重與資產分佈。
    /// 某個區塊沒有資料時保留舊資料（來源暫時缺漏不應把已知的持股清空）。
    async fn replace_portfolio(&self, portfolio: &EtfPortfolio) -> Result<u64>;

    /// 多檔 ETF 自 `from` 起的淨值序列（`close` 為淨值；正式淨值優先，當天未回填時用預估淨值），
    /// 依代號分組、由舊到新；沒有淨值的代號不在結果中。
    async fn nav_series(
        &self,
        stock_symbols: &[String],
        from: NaiveDate,
    ) -> Result<HashMap<String, Vec<PriceBar>>>;

    /// 寫入（或更正）績效；也刪除已下市 ETF 的績效。
    async fn save_performances(&self, performances: &[EtfPerformance]) -> Result<u64>;

    /// 重建清單頁快照 `etf_snapshot`（每檔一列），回傳寫入列數。
    async fn refresh_snapshot(&self) -> Result<u64>;

    /// 指定 ETF 中，最新一期前十大持股在 `recorded_since`（含）之後才收到、且有前一期可比的，
    /// 回傳兩期的持股。
    async fn holding_periods(
        &self,
        stock_symbols: &[String],
        recorded_since: NaiveDate,
    ) -> Result<Vec<EtfHoldingPeriods>>;

    /// 全部 ETF 最近 `days` 個有單位數的交易日的淨申購合計與折溢價；沒有單位數的歷史回補日不計。
    async fn weekly_flows(&self, days: i64) -> Result<Vec<EtfWeeklyFlow>>;

    /// 指定 ETF 最近一筆有單位數的受益單位變動，依代號分組；沒有的代號不在結果中。
    async fn latest_units(
        &self,
        stock_symbols: &[String],
    ) -> Result<HashMap<String, UnitsChangePoint>>;

    /// 指定 ETF 最近 `days` 筆折溢價，依代號分組、由新到舊。
    async fn recent_premiums(
        &self,
        stock_symbols: &[String],
        days: i64,
    ) -> Result<HashMap<String, Vec<PremiumPoint>>>;
}
