//! 測試專用：把指定來源（`scheme://host`）的請求改送到本機 mock server（G15）的測試。

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
