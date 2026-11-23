use crate::infra::nosql::redis::RedisError;
use thiserror::Error;

pub mod after_hours;
pub mod cagr_source;
pub mod chip;
pub mod chip_flow;
pub mod config;
pub mod corporate_action;
pub mod data_health;
pub mod dividend;
pub mod dividend_fill;
pub mod etf;
pub mod etf_query;
pub mod ex_right;
pub mod financial;
pub mod financial_health;
pub mod financial_report;
pub mod foreign_holding;
pub mod forward_return;
/// 持股籌碼日報（當日持股與法人、信用交易、主力進出）。
pub mod holding_chip;
pub mod holding_fundamentals;
// HoldingView → 會員編號清單。
pub mod holding_view;
pub mod market_index;
pub mod market_valuation;
/// 通知可見範圍內的會員名稱。
pub mod member;
pub mod money_flow;
// 通知項目目錄（notify_topic）的啟動寫入。
/// 通知送達紀錄的原子佔位與逾時對帳。
pub mod notify_delivery;
pub mod notify_route;
pub mod notify_topic;
pub mod official_ratio;
pub mod pb_percentile;
pub mod performance;
pub mod portfolio;
/// 持股穿透曝險的查詢（持有部位市值、持有 ETF 的前十大持股）。
pub mod portfolio_exposure;
pub mod portfolio_target;
pub mod quote;
pub mod relative_strength;
pub mod risk_metric;
pub mod stock;
pub mod strategy_preset;
pub mod strategy_rebalance;
pub mod technical;
pub mod trace;
pub mod valuation_band;
pub mod yield_rank;

/// 倉儲層結構化錯誤類型。
#[derive(Debug, Error)]
pub enum RepositoryError {
    /// 資料庫查詢或連線失敗。
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// 快取存取失敗。
    #[error("cache error: {0}")]
    Cache(#[from] RedisError),

    /// 指定實體不存在。
    #[error("entity not found")]
    NotFound,

    /// 序列化或反序列化失敗。
    #[error("serialization error: {0}")]
    Serialization(String),
}
