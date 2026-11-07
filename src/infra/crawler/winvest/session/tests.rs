//! Winvest antiforgery 工作階段的測試：cookie／token 解析、快取與換新。

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

/// 以本機 mock server 取代 Winvest 首頁，測工作階段的取得、快取與換新。
///
/// [`SESSION`] 是行程內的全域快取，每個測試開始前先清空、結束時再清空，避免互相影響
/// （CI 以單執行緒跑測試）。
mod mock_home {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;

    const ORIGIN: &str = "https://winvest.tw";

    /// 每次請求回一組新的 cookie 與 token（`token-1`、`token-2`…），並計數。
    struct RotatingHome(Arc<AtomicUsize>);

    impl Respond for RotatingHome {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            let n = self.0.fetch_add(1, Ordering::SeqCst) + 1;
            ResponseTemplate::new(200)
                .insert_header(
                    "set-cookie",
                    format!(".AspNetCore.Antiforgery.Test=cookie-{n}; path=/; httponly").as_str(),
                )
                .set_body_string(format!(
                    r#"<script>$('<input name="__RequestVerificationToken" type="hidden" value="token-{n}" />')</script>"#
                ))
        }
    }

    async fn home(server: &MockServer) -> Arc<AtomicUsize> {
        let calls = Arc::new(AtomicUsize::new(0));
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(RotatingHome(Arc::clone(&calls)))
            .mount(server)
            .await;
        calls
    }

    async fn reset() {
        *SESSION.lock().await = None;
    }

    /// 第一次向首頁取得並快取；之後沿用快取，不再請求首頁。
    #[tokio::test]
    async fn current_fetches_once_then_reuses_cache() {
        reset().await;
        let server = MockServer::start().await;
        let calls = home(&server).await;
        let uri = server.uri();

        let (first, second) = with_origins(&[(ORIGIN, &uri)], async {
            (current().await, current().await)
        })
        .await;
        reset().await;

        let first = first.expect("取得工作階段");
        assert_eq!(first.cookie, ".AspNetCore.Antiforgery.Test=cookie-1");
        assert_eq!(first.token, "token-1");
        assert_eq!(second.expect("沿用快取"), first);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// 被拒的是快取中那組才換新；別人已換好的就直接沿用，不重複請求首頁。
    #[tokio::test]
    async fn renew_replaces_rejected_session_only_once() {
        reset().await;
        let server = MockServer::start().await;
        let calls = home(&server).await;
        let uri = server.uri();

        let (original, renewed, reused) = with_origins(&[(ORIGIN, &uri)], async {
            let original = current().await.expect("取得工作階段");
            let renewed = renew(&original).await.expect("換新");
            // 另一個請求也拿舊的那組被拒：快取已不是舊的，直接沿用新的。
            let reused = renew(&original).await.expect("沿用別人換好的");
            (original, renewed, reused)
        })
        .await;
        reset().await;

        assert_eq!(original.token, "token-1");
        assert_eq!(renewed.token, "token-2");
        assert_eq!(reused, renewed);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// 首頁非 2xx、缺 cookie、缺 token 都回錯誤，且不寫入快取。
    #[tokio::test]
    async fn fetch_fails_without_success_cookie_or_token() {
        reset().await;
        let cookie = ".AspNetCore.Antiforgery.Test=cookie; path=/";
        let token = r#"<input name="__RequestVerificationToken" type="hidden" value="t" />"#;
        let cases = [
            ("非 2xx", ResponseTemplate::new(503), "HTTP 503"),
            (
                "缺 cookie",
                ResponseTemplate::new(200).set_body_string(token),
                "no antiforgery cookie",
            ),
            (
                "缺 token",
                ResponseTemplate::new(200)
                    .insert_header("set-cookie", cookie)
                    .set_body_string("<html></html>"),
                "no __RequestVerificationToken",
            ),
        ];
        for (case, response, expected) in cases {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/"))
                .respond_with(response)
                .mount(&server)
                .await;
            let uri = server.uri();

            let error = with_origins(&[(ORIGIN, &uri)], current())
                .await
                .expect_err(case);

            assert!(error.to_string().contains(expected), "{case}: {error:#}");
            assert!(SESSION.lock().await.is_none(), "{case}: 失敗不應寫入快取");
        }
    }
}
