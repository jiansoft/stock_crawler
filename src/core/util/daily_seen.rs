//! 「每個鍵每天只記一次」的紀錄表。
//!
//! 盤中輪詢會讓同一個狀況（某檔冷門股抓不到價、某站給錯價）每隔幾秒重複發生；
//! warn 只需要當天第一筆，其餘降為 debug，避免洗版又不會漏掉新狀況。

use std::collections::HashSet;

use chrono::NaiveDate;

/// 當天已出現過的鍵；換日時清空。
#[derive(Debug, Default)]
pub struct DailySeen {
    day: Option<NaiveDate>,
    keys: HashSet<String>,
}

impl DailySeen {
    /// 這個鍵今天是否第一次出現（是的話記下來）。
    pub fn first_today(&mut self, key: &str, today: NaiveDate) -> bool {
        if self.day != Some(today) {
            self.day = Some(today);
            self.keys.clear();
        }
        self.keys.insert(key.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同一個鍵同一天只算第一次，換日後重新計算。
    #[test]
    fn first_today_resets_on_day_change() {
        let day1 = NaiveDate::from_ymd_opt(2001, 1, 2).expect("valid date");
        let day2 = NaiveDate::from_ymd_opt(2001, 1, 3).expect("valid date");

        let mut seen = DailySeen::default();

        assert!(seen.first_today("79965", day1));
        assert!(!seen.first_today("79965", day1));
        assert!(seen.first_today("79966", day1));
        assert!(seen.first_today("79965", day2));
        assert!(!seen.first_today("79965", day2));
    }
}
