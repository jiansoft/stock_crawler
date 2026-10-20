//! Seq 日誌轉送的測試。
//!
//! 子模組：`events`（等級與序列化）、`endpoint`（網址安全檢查）、`flush`（批次送出）。

mod endpoint;
mod events;
mod flush;

use super::*;

/// 假 Seq 收到的每次請求：`(X-Seq-ApiKey, 內文)`。
type Requests = std::sync::Arc<std::sync::Mutex<Vec<(Option<String>, String)>>>;
