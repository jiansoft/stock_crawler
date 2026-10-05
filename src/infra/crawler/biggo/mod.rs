//! # BigGo 財經
//!
//! BigGo 財經（`finance.biggo.com.tw`）的頁面由前端呼叫 `api.biggo.com/api/v1/finance`
//! 取得資料。這支內部 API 匿名可用，回傳 `{"result": true, "data": …}`。用於：
//!
//! - Yahoo 三大財報失敗時的備援（[`financial_statement`]）。
//! - 即時報價備援池的站點之一（[`price`]）。
//!
//! ## 呼叫規則（2026-10 實測）
//!
//! - 代號一律帶 `.TW` 後綴（上櫃股也接受 `.TW`），並帶 `region=tw`。
//! - 參數錯誤回 HTTP 400 與 `{"result": false, "error": {"code": 1002, …}}`；
//!   查無資料回 `data: null`。
//! - 這是沒有文件的非官方 API，欄位可能無預警變動，因此解析採嚴格模式：
//!   格式不符就整批失敗，不猜測。

/// 三大財報（損益表、資產負債表、現金流量表）。
pub mod financial_statement;
/// 即時報價（即時報價備援池）。
pub mod price;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, de::DeserializeOwned};

use crate::core::util::{self, http};

/// BigGo 財經來源命名空間標記型別。
pub struct BigGo {}

/// BigGo 財經 API 的根網址。
const API_BASE: &str = "https://api.biggo.com/api/v1/finance";

/// BigGo 使用的代號格式（台股一律 `.TW`）。
fn api_symbol(stock_symbol: &str) -> String {
    format!("{stock_symbol}.TW")
}

/// BigGo API 的回應外層。
#[derive(Debug, Deserialize)]
struct Envelope<T> {
    /// 請求是否成功。
    result: bool,
    /// 回應資料；查無資料時為 `null`。
    data: Option<T>,
}

/// 呼叫 BigGo 財經 API 並取出 `data`。
///
/// `query` 是 `region=tw` 以外的查詢參數，值只會是代號與固定字串，不需另外編碼。
///
/// # Errors
///
/// HTTP 非 2xx、回應不是預期 JSON 或 `result` 為 `false` 時回傳錯誤。
async fn fetch<T: DeserializeOwned>(path: &str, query: &[(&str, &str)]) -> Result<Option<T>> {
    let mut url = format!("{API_BASE}{path}?region=tw");
    for (key, value) in query {
        url.push('&');
        url.push_str(key);
        url.push('=');
        url.push_str(value);
    }

    let response = http::get_response(&url, None).await?;
    let status = response.status();
    let body = response
        .text()
        .await
        .with_context(|| format!("Error reading BigGo response body from {url}"))?;
    if !status.is_success() {
        bail!(
            "BigGo {path} returned HTTP {status} for {url}. Body: {}",
            util::text::truncate(&body, 200)
        );
    }

    let envelope: Envelope<T> = serde_json::from_str(&body).with_context(|| {
        format!(
            "Error parsing BigGo {path} JSON from {url}. Body: {}",
            util::text::truncate(&body, 200)
        )
    })?;
    if !envelope.result {
        bail!(
            "BigGo {path} returned result=false for {url}. Body: {}",
            util::text::truncate(&body, 200)
        );
    }
    Ok(envelope.data)
}
