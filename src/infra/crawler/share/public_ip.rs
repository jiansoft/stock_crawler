//! # 公網 IP 查詢
//!
//! 以順序輪詢 (Round-robin) 的方式向多個第三方 IP 檢測服務查詢系統對外的公網 IP，
//! 並負責回應內容的正規化與各來源失敗原因的彙整。

use std::{
    net::IpAddr,
    sync::atomic::{AtomicUsize, Ordering},
};

use anyhow::Result;

use crate::infra::crawler::{CrawlerError, bigdatacloud, ipconfig, ipify, ipinfo, myip, seeip};

/// 全域 IP 查詢游標，用於順序輪詢不同的檢測服務。
static IP_INDEX: AtomicUsize = AtomicUsize::new(0);

/// 獲取系統對外的公網 IP 地址。
///
/// 此函式透過多個第三方 IP 檢測服務進行輪詢，以確保在單一服務失效時仍能取得 IP。
/// 為了平衡負載並避免單一服務請求過於頻繁，採用順序輪詢 (Round-robin) 機制切換不同站點。
///
/// # 支援的服務站點
/// - `ipify.org`
/// - `ipconfig.io`
/// - `ipinfo.io`
/// - `seeip.org`
/// - `myip.com`
/// - `bigdatacloud.com`
///
/// # 回傳值
/// - `Ok(String)`: 成功取得、且已正規化的公網 IP 字串。
/// - `Err`: 當所有嘗試的站點均失效時，回傳包含詳細錯誤原因的描述。
pub async fn get_public_ip() -> Result<String, CrawlerError> {
    const SERVICES: &[&str] = &[
        "ipify",
        "ipconfig",
        "ipinfo",
        "seeip",
        "myip",
        "bigdatacloud",
    ];

    let mut errors = Vec::with_capacity(SERVICES.len());

    for _ in 0..SERVICES.len() {
        let idx = IP_INDEX.fetch_add(1, Ordering::SeqCst) % SERVICES.len();
        let service_name = SERVICES[idx];

        let result = match service_name {
            "ipify" => ipify::visit().await,
            "ipconfig" => ipconfig::visit().await,
            "ipinfo" => ipinfo::visit().await,
            "seeip" => seeip::visit().await,
            "myip" => myip::visit().await,
            "bigdatacloud" => bigdatacloud::visit().await,
            _ => unreachable!(),
        };

        if let Some(ip) = collect_public_ip_result(service_name, result, &mut errors) {
            return Ok(ip);
        }
    }

    Err(CrawlerError::EmptyResponse(format!(
        "Failed to get public IP from all services: {}",
        errors.join(" | ")
    )))
}

/// 處理單一 IP 來源的回應結果。
///
/// 成功時會回傳已正規化的 IP；失敗時則把錯誤訊息附加到 `errors`，
/// 讓 `get_public_ip()` 最後能一次回報所有來源的失敗原因。
fn collect_public_ip_result(
    service_name: &str,
    result: Result<String>,
    errors: &mut Vec<String>,
) -> Option<String> {
    match result {
        Ok(ip) => match normalize_public_ip(service_name, &ip) {
            Ok(ip) => Some(ip),
            Err(why) => {
                errors.push(why.to_string());
                None
            }
        },
        Err(why) => {
            errors.push(format!("{service_name}: {why}"));
            None
        }
    }
}

/// 將第三方服務回傳的 IP 文字正規化成穩定格式。
///
/// 這裡會先去除前後空白，再要求內容必須能被解析為合法的
/// [`IpAddr`]；若解析成功，會回傳 `IpAddr::to_string()` 的標準化結果。
fn normalize_public_ip(service_name: &str, ip: &str) -> Result<String, CrawlerError> {
    let normalized = ip.trim();

    if normalized.is_empty() {
        return Err(CrawlerError::EmptyResponse(format!(
            "{service_name}: empty response"
        )));
    }

    normalized
        .parse::<IpAddr>()
        .map(|ip| ip.to_string())
        .map_err(|why| {
            CrawlerError::Parse(format!(
                "{service_name}: invalid ip response `{normalized}` because {why}"
            ))
        })
}

#[cfg(test)]
mod tests;
