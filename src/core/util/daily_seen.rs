//! 「每個鍵每天只記一次」的紀錄表。
//!
//! 盤中輪詢會讓同一個狀況（某檔冷門股抓不到價、某站給錯價）每隔幾秒重複發生；
//! warn 只需要當天第一筆，其餘降為 debug，避免洗版又不會漏掉新狀況。

use std::collections::HashSet;
use std::sync::RwLock;

use chrono::NaiveDate;

/// 當天已出現過的鍵；換日時清空。
#[derive(Debug, Default)]
pub struct DailySeen {
    day: Option<NaiveDate>,
    keys: HashSet<String>,
}

impl DailySeen {
    /// 只讀檢查，讓已出現過的鍵不必取得寫鎖。
    pub fn contains_today(&self, key: &str, today: NaiveDate) -> bool {
        self.day == Some(today) && self.keys.contains(key)
    }

    /// 這個鍵今天是否第一次出現（是的話記下來）。
    pub fn first_today(&mut self, key: &str, today: NaiveDate) -> bool {
        if self.day != Some(today) {
            self.day = Some(today);
            self.keys.clear();
        }
        self.keys.insert(key.to_string())
    }
}

/// 並行去重紀錄；重複鍵走讀鎖，新增與換日才取得寫鎖。
#[derive(Debug, Default)]
pub struct ConcurrentDailySeen(RwLock<DailySeen>);

impl ConcurrentDailySeen {
    /// 原子地判斷並記錄當天第一筆；鎖毒化時允許重新記錄警告。
    pub fn first_today(&self, key: &str, today: NaiveDate) -> bool {
        if self
            .0
            .read()
            .is_ok_and(|seen| seen.contains_today(key, today))
        {
            return false;
        }
        // 讀鎖釋放後重新檢查，避免兩個首次查詢同時通過。
        self.0
            .write()
            .map(|mut seen| seen.first_today(key, today))
            .unwrap_or(true)
    }
}

#[cfg(test)]
mod tests;
