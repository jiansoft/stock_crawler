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
pub mod ex_right;
pub mod financial;
pub mod financial_health;
pub mod financial_report;
pub mod foreign_holding;
pub mod forward_return;
/// 持股籌碼日報（當日持股與法人、信用交易、主力進出）。
pub mod holding_chip;
pub mod holding_fundamentals;
pub mod market_index;
pub mod market_valuation;
pub mod money_flow;
pub mod official_ratio;
pub mod performance;
pub mod portfolio;
pub mod quote;
pub mod relative_strength;
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
