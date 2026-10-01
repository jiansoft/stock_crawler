//! # Winvest antiforgery 工作階段
//!
//! 2026-09 改版後，Winvest 的查詢 API 採 ASP.NET Core antiforgery 驗證：
//! 請求必須同時帶上 `.AspNetCore.Antiforgery.*` cookie 與頁面內嵌的
//! `__RequestVerificationToken`（兩者成對，缺一即回 HTTP 400 空內容）。
//!
//! 同一組 cookie／token 可跨股票重複使用，因此這裡以行程內快取保存，
//! 逾時或請求被拒時才重新向首頁取得。

use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::{HeaderMap, SET_COOKIE};
use tokio::sync::Mutex;

use crate::{core::util, infra::crawler::winvest::HOST};

/// 工作階段的最長保存時間；逾時即重新取得，避免伺服器輪替金鑰後長時間失效。
const SESSION_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// antiforgery cookie 名稱的前綴（後綴為應用程式識別碼，改版時可能變動）。
const ANTIFORGERY_COOKIE_PREFIX: &str = ".AspNetCore.Antiforgery.";

/// 快取中的工作階段；`None` 代表尚未取得或已作廢。
static SESSION: Mutex<Option<Session>> = Mutex::const_new(None);

/// 比對 `name="__RequestVerificationToken" … value="…"` 的 input 片段。
///
/// token 只出現在 `<script>` 內以字串拼出的 hidden input
/// （`$('<input name="__RequestVerificationToken" type="hidden" value="…" />').val()`），
/// 不是真正的 DOM 節點，HTML parser 選不到，只能以文字比對。
static TOKEN_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"name="__RequestVerificationToken"[^>]*?value="([^"]+)""#)
        .expect("token pattern should compile")
});

/// 一組成對使用的 antiforgery cookie 與 token。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Session {
    /// 送回伺服器的 `Cookie` 標頭值（只含 `name=value`）。
    pub(super) cookie: String,
    /// `RequestVerificationToken` 標頭值。
    pub(super) token: String,
    /// 取得時間，用於判斷是否逾時。
    fetched_at: Instant,
}

/// 取得可用的工作階段；快取沒有或已逾時才重新取得。
pub(super) async fn current() -> Result<Session> {
    let mut cached = SESSION.lock().await;
    if let Some(session) = cached
        .as_ref()
        .filter(|session| session.fetched_at.elapsed() < SESSION_TTL)
    {
        return Ok(session.clone());
    }

    let session = fetch().await?;
    *cached = Some(session.clone());
    Ok(session)
}

/// 請求被拒後換一組新的工作階段。
///
/// 多個請求可能同時因同一組失效的 token 被拒；只有快取仍是 `rejected` 那組時才
/// 重新取得，其餘直接沿用別人剛換好的，避免同時向首頁重複請求。
pub(super) async fn renew(rejected: &Session) -> Result<Session> {
    let mut cached = SESSION.lock().await;
    if let Some(session) = cached
        .as_ref()
        .filter(|session| session.token != rejected.token)
    {
        return Ok(session.clone());
    }

    *cached = None;
    let session = fetch().await?;
    *cached = Some(session.clone());
    Ok(session)
}

/// 向 Winvest 首頁取得 antiforgery cookie 與內嵌 token。
async fn fetch() -> Result<Session> {
    let url = format!("https://{HOST}/");
    let response = util::http::get_response(&url, None).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!(
            "Failed to fetch Winvest antiforgery session from {url}: HTTP {status}"
        ));
    }

    let cookie = antiforgery_cookie(response.headers())
        .ok_or_else(|| anyhow!("Winvest response from {url} has no antiforgery cookie"))?;
    let html = response
        .text()
        .await
        .context("Failed to read Winvest home page")?;
    let token = extract_request_verification_token(&html)
        .ok_or_else(|| anyhow!("Winvest page {url} has no __RequestVerificationToken"))?;

    Ok(Session {
        cookie,
        token,
        fetched_at: Instant::now(),
    })
}

/// 從 `Set-Cookie` 取出 antiforgery cookie，只保留 `name=value`（去掉 path、httponly 等屬性）。
fn antiforgery_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .map(str::trim)
        .find(|pair| pair.starts_with(ANTIFORGERY_COOKIE_PREFIX) && pair.contains('='))
        .map(str::to_string)
}

/// 從頁面 HTML 取出 `__RequestVerificationToken` 的值，見 [`TOKEN_PATTERN`]。
fn extract_request_verification_token(html: &str) -> Option<String> {
    TOKEN_PATTERN
        .captures(html)
        .and_then(|captures| captures.get(1))
        .map(|token| token.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderValue;

    use super::*;

    #[test]
    fn antiforgery_cookie_keeps_only_name_value_pair() {
        let mut headers = HeaderMap::new();
        headers.append(
            SET_COOKIE,
            HeaderValue::from_static("WinvestWebLastUrlPath=%2F; path=/"),
        );
        headers.append(
            SET_COOKIE,
            HeaderValue::from_static(
                ".AspNetCore.Antiforgery.PczYmD31X3Y=CfDJ8abc-_x; path=/; samesite=strict; httponly",
            ),
        );

        assert_eq!(
            antiforgery_cookie(&headers).as_deref(),
            Some(".AspNetCore.Antiforgery.PczYmD31X3Y=CfDJ8abc-_x")
        );
    }

    #[test]
    fn antiforgery_cookie_is_none_without_antiforgery_cookie() {
        let mut headers = HeaderMap::new();
        headers.append(
            SET_COOKIE,
            HeaderValue::from_static("WinvestWebLastUrlPath=%2F; path=/"),
        );

        assert_eq!(antiforgery_cookie(&headers), None);
    }

    /// 頁面實際的寫法：token 藏在 `<script>` 內以 jQuery 字串拼出的 hidden input。
    #[test]
    fn extract_token_from_script_embedded_input() {
        let html = r#"<script>
            $.DoAjax({
                data: { inModel: postData, __RequestVerificationToken: $('<input name="__RequestVerificationToken" type="hidden" value="CfDJ8Ocb-Zd_x" />').val() },
            });
        </script>"#;

        assert_eq!(
            extract_request_verification_token(html).as_deref(),
            Some("CfDJ8Ocb-Zd_x")
        );
    }

    #[test]
    fn extract_token_is_none_when_missing_or_empty() {
        assert_eq!(extract_request_verification_token("<html></html>"), None);
        assert_eq!(
            extract_request_verification_token(
                r#"<input name="__RequestVerificationToken" type="hidden" value="" />"#
            ),
            None
        );
    }
}
