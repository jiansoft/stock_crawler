//! # 具名策略
//!
//! 一組條件選股回測的條件（與 `/stocks/screen/backtest` 相同的 query string）存成名稱，Data API 的回測
//! 端點以 `preset=名稱` 引用，`alert` 為 true 的策略由策略換股通知逐一執行。

use anyhow::Result;
use async_trait::async_trait;

/// 策略換股紀錄（換股通知當天的持股，供追蹤實際表現）。
pub mod rebalance;

/// 策略名稱的長度上限（字元）。
pub const MAX_NAME_CHARS: usize = 64;

/// 一個具名策略。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyPreset {
    /// 策略名稱。
    pub name: String,
    /// 條件選股回測的 query string（不含 `preset`）。
    pub query: String,
    /// 說明。
    pub description: String,
    /// 是否在策略換股通知中執行。
    pub alert: bool,
}

/// 檢查策略名稱：去掉前後空白後 1–[`MAX_NAME_CHARS`] 字元，不含 query string 的保留字元
/// （`&`、`=`、`#`、`?`）與控制字元。回傳去掉前後空白的名稱。
///
/// # Errors
///
/// 不符合規則時回傳說明訊息。
pub fn normalize_name(name: &str) -> Result<String, &'static str> {
    let name = name.trim();
    let chars = name.chars().count();
    if chars == 0 || chars > MAX_NAME_CHARS {
        return Err("策略名稱必須為 1 至 64 個字元");
    }
    if name
        .chars()
        .any(|c| matches!(c, '&' | '=' | '#' | '?') || c.is_control())
    {
        return Err("策略名稱不可包含 & = # ? 或控制字元");
    }
    Ok(name.to_owned())
}

/// 具名策略的存取。
#[async_trait]
pub trait StrategyPresetRepository: Send + Sync {
    /// 全部策略，依名稱排序。
    async fn list(&self) -> Result<Vec<StrategyPreset>>;

    /// 指定名稱的策略；不存在回 `None`。
    async fn find(&self, name: &str) -> Result<Option<StrategyPreset>>;

    /// 新增或覆蓋（依名稱）。
    async fn save(&self, preset: &StrategyPreset) -> Result<()>;

    /// 刪除；回傳是否真的刪除了一筆。
    async fn delete(&self, name: &str) -> Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 名稱去掉前後空白、長度 1–64 字元（以字元計，中文也算一個），不可含 query string 保留字元。
    #[test]
    fn normalize_name_trims_and_rejects_reserved() {
        assert_eq!(normalize_name("  最佳組合 "), Ok("最佳組合".to_owned()));
        assert_eq!(
            normalize_name(&"甲".repeat(64)).map(|name| name.chars().count()),
            Ok(64)
        );
        assert!(normalize_name(&"甲".repeat(65)).is_err());
        assert!(normalize_name("   ").is_err());
        for name in ["a&b", "a=b", "a#b", "a?b", "a\nb"] {
            assert!(normalize_name(name).is_err(), "{name}");
        }
    }
}
