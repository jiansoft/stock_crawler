use chrono::{Datelike, Local, Weekday};

// 自定義的 Weekend trait
pub trait Weekend {
    fn is_weekend(&self) -> bool;
}

// 為 chrono::Date<Local> 實現 Weekend trait
impl Weekend for chrono::DateTime<Local> {
    /// 星期六、星期日視為假日
    fn is_weekend(&self) -> bool {
        matches!(self.weekday(), Weekday::Sat | Weekday::Sun)
    }
}

/// 月份轉季度
pub fn month_to_quarter(month: u32) -> &'static str {
    match month {
        1..=3 => "Q1",
        4..=6 => "Q2",
        7..=9 => "Q3",
        10..=12 => "Q4",
        _ => "Invalid month",
    }
}
