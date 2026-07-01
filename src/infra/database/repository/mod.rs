use crate::infra::nosql::redis::RedisError;
use thiserror::Error;

pub mod config;
pub mod dividend;
pub mod financial;
pub mod market_index;
pub mod money_flow;
pub mod portfolio;
pub mod quote;
pub mod stock;
pub mod trace;
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
