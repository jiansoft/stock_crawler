use anyhow::{anyhow, Result};
use chrono::NaiveDate;

/// 代表系統設定（鍵值對）的領域實體。
///
/// 用於持久化記錄系統執行狀態、最後回補日期、外部排程進度等設定數據。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemConfig {
    /// 設定鍵名，例如 "last_revenue_backfill_date"
    pub key: String,
    /// 設定值，以字串形式儲存
    pub val: String,
}

impl SystemConfig {
    /// 建立全新系統設定實體的工廠方法。
    ///
    /// # 參數
    /// * `key` - 設定鍵
    /// * `val` - 設定值
    pub fn new(key: String, val: String) -> Self {
        Self { key, val }
    }

    /// 嘗試將設定值解析為日期格式 (`%Y-%m-%d`)。
    ///
    /// # Errors
    /// 當設定值無法正確解析為日期時回傳錯誤。
    pub fn parse_val_as_date(&self) -> Result<NaiveDate> {
        NaiveDate::parse_from_str(&self.val, "%Y-%m-%d").map_err(|why| {
            anyhow!(
                "Failed to parse config val '{}' as date: {:?}",
                self.val,
                why
            )
        })
    }

    /// 比較傳入的日期與當前設定值中的日期，判斷是否應進行更新。
    ///
    /// 只有在傳入的 `new_date` 晚於（大於）當前設定值中的日期時，才傳回 `true` 代表應該更新。
    /// 若當前設定值不合法（無法解析為日期），亦視為需要更新。
    ///
    /// # 參數
    /// * `new_date` - 新的交易/回補日期。
    pub fn should_update_date(&self, new_date: NaiveDate) -> bool {
        match self.parse_val_as_date() {
            Ok(current_date) => new_date > current_date,
            Err(_) => true, // 原值不合法時強制允許更新
        }
    }
}
