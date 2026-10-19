//! Application 層對外部服務的抽象介面（ports）。
//!
//! ## 為什麼需要這個模組
//!
//! 依 DDD 分層，`app`（use case）不應該直接依賴 `interfaces` 層的
//! gRPC 產生碼（prost DTO）——那是傳輸層細節，一旦 proto 變更或想改用
//! HTTP/訊息佇列推送，所有 use case 都得跟著改，也無法在單元測試中替換。
//!
//! ## 解法：port / adapter
//!
//! - 這裡定義與傳輸層無關的資料載體與 gateway trait（port）。
//! - 具體實作（gRPC client）由 `interfaces::rpc::client` 提供（adapter），
//!   並在 `main` 啟動時透過 [`register_stock_info_gateway`] 注入。
//! - 未註冊時（例如單元測試）推送會被略過並記 warning，不影響主流程。
//!
//! 策略換股通知（[`StrategyGateway`]）也用同樣的方式：回測引擎在 `interfaces::web::data_api`，
//! 由 `main` 注入，use case 只拿到與傳輸層無關的換股結果。

use std::sync::{Arc, OnceLock};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::NaiveDate;

/// 與傳輸層無關的股票基本資料載體。
///
/// 欄位刻意只用基本型別，不引用任何 prost 產生的 DTO。
#[derive(Debug, Clone)]
pub struct StockInfoPush {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 交易市場別 ID。
    pub stock_exchange_market_id: i32,
    /// 產業分類 ID。
    pub stock_industry_id: i32,
}

/// 將股票基本資料推送到外部服務的 gateway（port）。
#[async_trait]
pub trait StockInfoGateway: Send + Sync {
    /// 推送單筆股票基本資料。
    async fn push_stock_info(&self, info: StockInfoPush) -> Result<()>;
}

/// 全域 gateway 實例。`OnceLock` 保證只會被成功註冊一次。
static STOCK_INFO_GATEWAY: OnceLock<Arc<dyn StockInfoGateway>> = OnceLock::new();

/// 註冊股票資料推送 gateway，應於 `main` 啟動流程呼叫一次。
pub fn register_stock_info_gateway(gateway: Arc<dyn StockInfoGateway>) {
    if STOCK_INFO_GATEWAY.set(gateway).is_err() {
        tracing::warn!("stock info gateway already registered; duplicate registration ignored");
    }
}

/// 透過已註冊的 gateway 推送股票基本資料。
///
/// 尚未註冊時（例如單元測試）記 warning 後直接返回——
/// 推送屬「盡力同步」性質，不應讓主要事件流程失敗。
pub async fn push_stock_info(info: StockInfoPush) {
    match STOCK_INFO_GATEWAY.get() {
        Some(gateway) => {
            let symbol = info.stock_symbol.clone();
            if let Err(why) = gateway.push_stock_info(info).await {
                tracing::error!("Failed to push stock info for {symbol} because {why:?}");
            }
        }
        None => {
            tracing::warn!(
                "stock info gateway not registered; push skipped for {}",
                info.stock_symbol
            );
        }
    }
}

/// 策略換股的一檔股票。
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyHolding {
    /// 股票代號。
    pub symbol: String,
    /// 股票名稱；查無為 `None`。
    pub name: Option<String>,
    /// 在策略部位中的權重（%）；賣出的股票為上一期換股時的權重。
    pub weight: f64,
    /// 換股日收盤到最新收盤的還原報酬（%）；賣出的股票或查不到日線為 `None`。
    pub return_to_date: Option<f64>,
}

/// 依策略條件最後一次換股的結果。
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyRebalance {
    /// 換股日。
    pub date: NaiveDate,
    /// 持有天數（交易日），也就是距下次換股的交易日數。
    pub hold_days: u16,
    /// 換股日之後到最新交易日經過的交易日數（換股當天為 0）。
    pub days_elapsed: i64,
    /// 0050 核心部位（%）；0 為純策略。
    pub etf_weight: u8,
    /// 大盤濾網沒過、策略部位空手。
    pub market_filtered: bool,
    /// 策略部位放現金的比例（%）。
    pub cash: f64,
    /// 相對上一期新買進與加碼的部位比例（0–1）。
    pub turnover: f64,
    /// 策略部位換股日收盤至今的還原報酬（%，依權重、未扣成本）；查不到為 `None`。
    pub return_to_date: Option<f64>,
    /// 0050 同期的還原報酬（%）；查不到為 `None`。
    pub etf_return_to_date: Option<f64>,
    /// 新買進的股票，依排序由前到後。
    pub added: Vec<StrategyHolding>,
    /// 續抱的股票，依排序由前到後。
    pub kept: Vec<StrategyHolding>,
    /// 上一期持有、這次賣出的股票。
    pub exited: Vec<StrategyHolding>,
}

/// 依換股紀錄算出的上一期實盤結算與追蹤以來的累積表現（還原報酬、未扣成本）。
#[derive(Debug, Clone, PartialEq)]
pub struct StrategySettlement {
    /// 上一期的換股日。
    pub start: NaiveDate,
    /// 上一期的結束日（這次換股日或最新交易日）。
    pub end: NaiveDate,
    /// 上一期策略部位的報酬（%）；查不到日線為 `None`。
    pub strategy_return: Option<f64>,
    /// 上一期 0050 的報酬（%）。
    pub etf_return: Option<f64>,
    /// 上一期組合（策略部位與 0050 核心部位依比例混合）的報酬（%）。
    pub portfolio_return: Option<f64>,
    /// 上一期的 0050 核心部位（%）。
    pub etf_weight: u8,
    /// 第一次記錄的換股日。
    pub tracked_from: NaiveDate,
    /// 追蹤以來已結束的期數（含上一期）。
    pub periods: usize,
    /// 追蹤以來各期組合報酬連乘的累積報酬（%）；有任一期查不到為 `None`。
    pub cumulative_return: Option<f64>,
    /// 同一段期間持有 0050 的報酬（%）。
    pub etf_cumulative_return: Option<f64>,
}

/// 依條件選股回測的條件取得策略最後一次換股的 gateway（port）。
#[async_trait]
pub trait StrategyGateway: Send + Sync {
    /// 以 `query`（條件選股回測的 query string）跑一次回測，回傳最後一次換股、還沒滿持有天數的
    /// 持股；最後一期剛好滿持有天數（今天沒有持股資料）時回 `None`。
    async fn latest_rebalance(&self, query: &str) -> Result<Option<StrategyRebalance>>;

    /// 依 `preset_name` 的換股紀錄，結算 `date` 之前最後一次換股的那一期（持有到 `date` 或最新交易日）
    /// 與追蹤以來的累積表現；`date` 之前沒有紀錄時回 `None`。
    async fn settlement(
        &self,
        preset_name: &str,
        date: NaiveDate,
    ) -> Result<Option<StrategySettlement>>;
}

/// 全域策略 gateway。
static STRATEGY_GATEWAY: OnceLock<Arc<dyn StrategyGateway>> = OnceLock::new();

/// 註冊策略 gateway，應於 `main` 啟動流程呼叫一次。
pub fn register_strategy_gateway(gateway: Arc<dyn StrategyGateway>) {
    if STRATEGY_GATEWAY.set(gateway).is_err() {
        tracing::warn!("strategy gateway already registered; duplicate registration ignored");
    }
}

/// 已註冊的策略 gateway。
///
/// # Errors
///
/// 尚未註冊時回傳錯誤。
pub fn strategy_gateway() -> Result<Arc<dyn StrategyGateway>> {
    STRATEGY_GATEWAY
        .get()
        .cloned()
        .ok_or_else(|| anyhow!("strategy gateway not registered"))
}
