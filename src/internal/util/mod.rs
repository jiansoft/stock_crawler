pub mod http;
pub mod text;

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
