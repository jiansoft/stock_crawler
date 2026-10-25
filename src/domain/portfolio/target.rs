//! # 投資組合目標配置（Portfolio Target）
//!
//! 定義使用者在個股、產業、ETF穿透三種維度下的目標配置比例。

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 配置維度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetDimension {
    /// 個股維度（key 為股票代號）
    Stock,
    /// 產業維度（key 為產業名稱）
    Industry,
    /// ETF 穿透維度（key 為股票代號）
    EtfLookThrough,
}

impl TargetDimension {
    /// 維度名稱字串。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stock => "stock",
            Self::Industry => "industry",
            Self::EtfLookThrough => "etf_look_through",
        }
    }

    /// 從字串解析維度；不認得回 `None`。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "stock" => Some(Self::Stock),
            "industry" => Some(Self::Industry),
            "etf_look_through" => Some(Self::EtfLookThrough),
            _ => None,
        }
    }
}

/// 單筆目標配置紀錄。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortfolioTargetRecord {
    /// 使用者代號或會員 ID。
    pub member_id: String,
    /// 配置維度。
    pub dimension: TargetDimension,
    /// 維度鍵值（股票代號或產業名稱）。
    pub key: String,
    /// 目標百分比（0.00 ~ 100.00）。
    pub target_percent: f64,
    /// 最後更新時間。
    pub updated_time: DateTime<Utc>,
}

/// 投資組合目標配置倉儲介面。
#[async_trait]
pub trait PortfolioTargetRepository: Send + Sync {
    /// 取得指定使用者在指定維度（若為 None 則為全維度）的目標配置列表。
    async fn list_targets(
        &self,
        member_id: &str,
        dimension: Option<TargetDimension>,
    ) -> Result<Vec<PortfolioTargetRecord>>;

    /// 設定指定使用者在指定維度的目標配置列表（批次覆蓋該維度的舊設定）。
    ///
    /// 同一維度的 target_percent 總和若超過 100.0 應拒絕（回傳 Err）。
    async fn save_targets(
        &self,
        member_id: &str,
        dimension: TargetDimension,
        targets: &[(String, f64)],
    ) -> Result<()>;
}
