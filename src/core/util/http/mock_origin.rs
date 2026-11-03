//! 測試專用：把指定來源（`scheme://host`）的請求改送到本機 mock server（G15）。
//!
//! 爬蟲的網址都寫死成正式站（`https://www.twse.com.tw/...`），連外的排程入口因此測不到。
//! 所有請求最後都經過 [`super::send::send_with_client`]，在那裡依 [`with_origins`] 設定的對照表
//! 把網址開頭換掉，爬蟲本身不用改簽名就能改打 `wiremock` 的假站。
//!
//! 對照表存在 task-local：只影響 [`with_origins`] 包住的那個 future（`tokio::join!` 的分支也算），
//! 平行執行的其他測試不受影響；但 `tokio::spawn` 出去的新 task 看不到，這類流程要在被測函式內
//! 直接 await 的範圍才會改寫。

use std::borrow::Cow;
use std::future::Future;

tokio::task_local! {
    /// 正式站來源 → mock server 基底網址。
    static ORIGINS: Vec<(String, String)>;
}

/// 在 `fut` 執行期間，把以 `origins` 中任一正式站來源（例如 `https://www.twse.com.tw`）開頭的
/// 請求改送到對應的 mock server 基底網址（例如 `MockServer::uri()`）。
pub(crate) async fn with_origins<F: Future>(origins: &[(&str, &str)], fut: F) -> F::Output {
    let origins = origins
        .iter()
        .map(|(origin, base)| ((*origin).to_owned(), (*base).to_owned()))
        .collect();
    ORIGINS.scope(origins, fut).await
}

/// 依目前 task 的對照表改寫 `url`；沒有設定或沒有符合的來源時原樣回傳。
///
/// 來源後面必須緊接 `/`、`?` 或字串結尾，避免 `https://a.com` 誤配到 `https://a.com.tw`。
pub(super) fn rewrite(url: &str) -> Cow<'_, str> {
    ORIGINS
        .try_with(|origins| {
            origins.iter().find_map(|(origin, base)| {
                let rest = url.strip_prefix(origin.as_str())?;
                matches!(rest.chars().next(), None | Some('/' | '?'))
                    .then(|| Cow::Owned(format!("{base}{rest}")))
            })
        })
        .ok()
        .flatten()
        .unwrap_or(Cow::Borrowed(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只改寫符合的來源；路徑、query 保留，相似網域與沒有設定時原樣回傳。
    #[tokio::test]
    async fn rewrite_replaces_matching_origin_only() {
        assert_eq!(
            rewrite("https://www.twse.com.tw/a"),
            "https://www.twse.com.tw/a"
        );

        let rewritten = with_origins(
            &[("https://www.twse.com.tw", "http://127.0.0.1:1")],
            async {
                [
                    "https://www.twse.com.tw/rwd/zh/x?date=2026",
                    "https://www.twse.com.tw?x=1",
                    "https://www.twse.com.tw",
                    "https://www.twse.com.tw.evil/a",
                    "https://www.tpex.org.tw/a",
                ]
                .map(|url| rewrite(url).into_owned())
            },
        )
        .await;

        assert_eq!(
            rewritten,
            [
                "http://127.0.0.1:1/rwd/zh/x?date=2026",
                "http://127.0.0.1:1?x=1",
                "http://127.0.0.1:1",
                "https://www.twse.com.tw.evil/a",
                "https://www.tpex.org.tw/a",
            ]
        );
    }
}
