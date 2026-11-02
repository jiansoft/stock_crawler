//! POST 請求：JSON 請求與回應（`post_use_json`）、表單或自訂內容（`post`）。

use std::collections::HashMap;

use anyhow::{Context, Result};
use reqwest::{Method, RequestBuilder, header};
use serde::{Serialize, de::DeserializeOwned};

use super::send::{format_form_params_log, send};
use super::{MAX_RESPONSE_BODY_BYTES, read_limited_body, redact_url};
use crate::core::util;

/// Performs an HTTP POST request with JSON request and response, and specified headers.
///
/// # Type Parameters
///
/// * `REQ`: The request type to serialize as JSON. It must implement `Serialize`.
/// * `RES`: The response type to deserialize from JSON. It must implement `DeserializeOwned`.
///
/// # Arguments
///
/// * `url`: The URL to send the POST request to.
/// * `headers`: An optional set of headers to include with the request.
/// * `req`: An optional reference to the request object to be serialized as JSON.
///
/// # Returns
///
/// * `Result<RES>`: The deserialized response, or an error if the request fails or the response cannot be deserialized.
pub async fn post_use_json<REQ, RES>(
    url: &str,
    headers: Option<header::HeaderMap>,
    req: Option<&REQ>,
) -> Result<RES>
where
    REQ: Serialize,
    RES: DeserializeOwned,
{
    let res = send(
        Method::POST,
        url,
        headers,
        Some(
            |rb: RequestBuilder| {
                if let Some(r) = req { rb.json(r) } else { rb }
            },
        ),
        None,
    )
    .await?;

    // 以「串流 + 大小上限」讀取 body，取代無上限的 res.text()。
    let res_body = read_limited_body(res, url, MAX_RESPONSE_BODY_BYTES).await?;
    let res_body_preview = String::from_utf8_lossy(res_body.as_ref());

    // with_context 保留 serde 原始錯誤在 source chain。
    // 錯誤訊息中的 body 改為固定長度片段（舊版會把「整份 body」塞進錯誤，
    // 造成大型 log，回應含敏感內容時也可能整段被寫進日誌）。
    serde_json::from_slice(res_body.as_ref()).with_context(|| {
        format!(
            "Error parsing response JSON from {}. Body: {}",
            redact_url(url),
            util::text::log_snippet(&res_body_preview, 200)
        )
    })
}

/// Performs an HTTP POST request with form data and specified headers, and returns the response as text.
///
/// # Arguments
///
/// * `url`: The URL to send the POST request to.
/// * `headers`: An optional set of headers to include with the request.
/// * `params`: An optional map of form data key-value pairs.
///
/// # Returns
///
/// * `Result<String>`: The response text, or an error if the request fails
///   or the response cannot be parsed.
pub async fn post(
    url: &str,
    headers: Option<header::HeaderMap>,
    params: Option<HashMap<&str, &str>>,
) -> Result<String> {
    let body_fn: Option<fn(RequestBuilder) -> RequestBuilder> = None;
    let response = match params {
        Some(p) => {
            let request_detail = format_form_params_log(&p);
            send(
                Method::POST,
                url,
                headers,
                Some(move |rb: RequestBuilder| rb.form(&p)),
                Some(request_detail),
            )
            .await?
        }
        None => send(Method::POST, url, headers, body_fn, None).await?,
    };

    // context 保留 reqwest 原始錯誤在 source chain。
    response.text().await.context("Error parsing response text")
}
