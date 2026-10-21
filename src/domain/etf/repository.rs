use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;

use super::{EtfNavRecord, EtfPerformance, EtfPortfolio, EtfProfileRecord, PremiumPoint};

/// ETF 專區資料的倉儲合約。寫入方法回傳實際新增或變更的列數。
#[async_trait]
pub trait EtfRepository: Send + Sync {
    /// 目前上市櫃中的 ETF（產業 9001、未下市）代號與名稱，依代號排序。
    async fn listed_etfs(&self) -> Result<Vec<(String, String)>>;

    /// 寫入（或更正）淨值資料，並以每筆的「前一營業日單位淨值」回填該 ETF 上一筆的正式淨值。
    async fn save_navs(&self, records: &[EtfNavRecord]) -> Result<u64>;

    /// 寫入（或更正）基本資料；不動 `manual_override`。
    async fn save_profiles(&self, profiles: &[EtfProfileRecord]) -> Result<u64>;

    /// 為還沒有基本資料的 ETF 補上推斷的分類；先前推斷的列跟著最新規則更新，來源（Yahoo）寫入的列不動。
    async fn insert_missing_profiles(&self, profiles: &[EtfProfileRecord]) -> Result<u64>;

    /// 以最新一期取代一檔 ETF 的前十大持股、行業比重與資產分佈。
    /// 某個區塊沒有資料時保留舊資料（來源暫時缺漏不應把已知的持股清空）。
    async fn replace_portfolio(&self, portfolio: &EtfPortfolio) -> Result<u64>;

    /// 寫入（或更正）績效；也刪除已下市 ETF 的績效。
    async fn save_performances(&self, performances: &[EtfPerformance]) -> Result<u64>;

    /// 重建清單頁快照 `etf_snapshot`（每檔一列），回傳寫入列數。
    async fn refresh_snapshot(&self) -> Result<u64>;

    /// 指定 ETF 最近 `days` 筆折溢價，依代號分組、由新到舊。
    async fn recent_premiums(
        &self,
        stock_symbols: &[String],
        days: i64,
    ) -> Result<HashMap<String, Vec<PremiumPoint>>>;
}
