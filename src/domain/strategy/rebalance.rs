//! # 策略換股紀錄
//!
//! 策略換股通知遇到換股日時存下當天通知的持股，之後逐期追蹤實際表現。紀錄只寫一次，之後重跑回測或
//! 修改策略條件都不會改寫歷史，所以追蹤到的是「當時實際照著做」的結果。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

/// 換股當天策略部位的一檔持股。
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedHolding {
    /// 股票代號。
    pub symbol: String,
    /// 股票名稱；查無為 `None`。
    pub name: Option<String>,
    /// 在策略部位的權重（%）。
    pub weight: f64,
}

/// 一次換股的紀錄。
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyRebalanceRecord {
    /// 策略名稱。
    pub preset_name: String,
    /// 換股日（收盤買進）。
    pub date: NaiveDate,
    /// 當時的回測條件（query string）。
    pub query: String,
    /// 持有天數（交易日）。
    pub hold_days: u16,
    /// 0050 核心部位（%）；0 為純策略。
    pub etf_weight: u8,
    /// 策略部位放現金的比例（%）。
    pub cash: f64,
    /// 大盤濾網沒過、策略部位空手。
    pub market_filtered: bool,
    /// 策略部位持股，依排序由前到後。
    pub holdings: Vec<RecordedHolding>,
}

/// 策略換股紀錄的存取。
#[async_trait]
pub trait StrategyRebalanceRepository: Send + Sync {
    /// 寫入一次換股；同一策略同一天已有紀錄時保留原本的，回傳 `false`。
    async fn save(&self, record: &StrategyRebalanceRecord) -> Result<bool>;

    /// 指定策略的全部換股紀錄，依換股日由舊到新。
    async fn list(&self, preset_name: &str) -> Result<Vec<StrategyRebalanceRecord>>;
}
