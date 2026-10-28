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

/// 目標配置的一個歷史版本：某次儲存前的整組配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortfolioTargetRevision {
    /// 版本編號。
    pub id: i64,
    /// 配置維度。
    pub dimension: TargetDimension,
    /// 那一組配置被取代（記成這一版）的時間。
    pub saved_time: DateTime<Utc>,
    /// 那一組配置的 `(key, 目標百分比)`，依百分比由高到低；空的代表全現金。
    pub targets: Vec<(String, f64)>,
}

/// 每個會員每個維度保留的歷史版本數。
pub const MAX_TARGET_REVISIONS: i64 = 20;

/// 投資組合目標配置倉儲介面。
#[async_trait]
pub trait PortfolioTargetRepository: Send + Sync {
    /// 取得指定使用者在指定維度（若為 None 則為全維度）的目標配置列表。
    async fn list_targets(
        &self,
        member_id: &str,
        dimension: Option<TargetDimension>,
    ) -> Result<Vec<PortfolioTargetRecord>>;

    /// 區分尚未設定與明確設定為全現金（空清單）。
    async fn has_configuration(&self, member_id: &str, dimension: TargetDimension) -> Result<bool>;

    /// 設定指定使用者在指定維度的目標配置列表（批次覆蓋該維度的舊設定）。
    ///
    /// 同一維度的 target_percent 總和若超過 100.0 應拒絕（回傳 Err）。該維度原本已設定且內容有變時，
    /// 實作應把覆蓋前的整組記成一版（[`PortfolioTargetRevision`]），供查詢與還原。
    async fn save_targets(
        &self,
        member_id: &str,
        dimension: TargetDimension,
        targets: &[(String, f64)],
    ) -> Result<()>;
}

/// 驗證輸入與儲存精度一致：只接受兩位小數、有限值與不重複的非空 key。
/// 以整數基點加總，避免資料庫逐項四捨五入後超過 100%。
pub fn validate_targets(targets: &[(String, f64)]) -> Result<()> {
    let mut keys = std::collections::HashSet::new();
    let mut basis_points = 0_i64;
    for (key, percent) in targets {
        anyhow::ensure!(
            !key.trim().is_empty() && key.chars().count() <= 64 && keys.insert(key),
            "目標 key 不可空白、重複或超過 64 字元"
        );
        anyhow::ensure!(
            percent.is_finite() && (0.0..=100.0).contains(percent),
            "目標比例須為 0 至 100 的有限數值"
        );
        let scaled = percent * 100.0;
        anyhow::ensure!(
            (scaled - scaled.round()).abs() < 1e-7,
            "目標比例最多兩位小數"
        );
        basis_points += scaled.round() as i64;
    }
    anyhow::ensure!(
        basis_points <= 10_000,
        "同一維度的目標權重總和不得超過 100%"
    );
    Ok(())
}
