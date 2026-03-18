//! 通用工具模組。

use std::{cmp::max, sync::Once};

/// 日期時間相關工具。
pub mod datetime;
/// 執行期 diagnostics 與程序狀態工具。
pub mod diagnostics;
/// HTTP 請求與 HTML 解析輔助工具。
pub mod http;
/// 集合與 map 轉換工具。
pub mod map;
/// 文字與數值解析工具。
pub mod text;

/// 文數字間的轉換
pub mod convert;
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
/// 取得建議的 16 級併發上限。
pub fn concurrent_limit_16() -> Option<usize> {
    Some(max(16, num_cpus::get() * 4))
}

/// 取得建議的 32 級併發上限。
pub fn concurrent_limit_32() -> Option<usize> {
    Some(max(32, num_cpus::get() * 4))
}

/// 取得建議的 64 級併發上限。
pub fn concurrent_limit_64() -> Option<usize> {
    Some(max(64, num_cpus::get() * 4))
}

static RUSTLS_CRYPTO_PROVIDER: Once = Once::new();

/// Ensure rustls has a process-wide crypto provider before any TLS client/server is built.
pub fn ensure_rustls_crypto_provider() {
    RUSTLS_CRYPTO_PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
