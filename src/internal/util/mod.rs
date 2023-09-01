use std::cmp::max;

pub mod datetime;
pub mod http;
pub mod text;
pub mod map;
/*
分錢算式有小數位
fn distribute_amount(amount: f64, parts: usize) -> Vec<f64> {
    let mut result = vec![0.0; parts];
    let mut remaining = amount;

    for i in 0..parts {
        let share = remaining / (parts - i) as f64;
        result[i] = (share * 1e4).round() / 1e4; // Round to 4 decimal places
        remaining -= result[i];
    }

    result
}
分錢算式無小數位
fn distribute_amount(amount: i32, parts: usize) -> Vec<i32> {
    let mut result = vec![0; parts];
    let mut remaining = amount;

    for i in 0..parts {
        let share = remaining as f64 / (parts - i) as f64;
        let rounded_share = share.round() as i32;
        result[i] = rounded_share;
        remaining -= rounded_share;
    }

    result
}

*/

pub fn concurrent_limit_16() -> Option<usize> {
    Some(max(16, num_cpus::get() * 4))
}

pub fn concurrent_limit_32() -> Option<usize> {
    Some(max(32, num_cpus::get() * 4))
}

pub fn concurrent_limit_64() -> Option<usize> {
    Some(max(64, num_cpus::get() * 4))
}
