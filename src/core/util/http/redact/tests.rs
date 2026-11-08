//! 日誌與錯誤訊息的敏感資訊遮罩（Telegram Bot Token）的測試。

use super::*;

#[test]
fn test_redact_url() {
    let normal_url = "https://www.twse.com.tw/exchangeReport/FMTQIK?response=json";
    assert_eq!(redact_url(normal_url), normal_url);

    let tg_url_with_path =
        "https://api.telegram.org/bot123456789:ABCdefGhIJKlmNoPQRsTUVwxyZ/sendMessage";
    let expected_path = "https://api.telegram.org/bot<redacted>/sendMessage";
    assert_eq!(redact_url(tg_url_with_path), expected_path);

    let tg_url_no_path = "https://api.telegram.org/bot123456789:ABCdefGhIJKlmNoPQRsTUVwxyZ";
    let expected_no_path = "https://api.telegram.org/bot<redacted>";
    assert_eq!(redact_url(tg_url_no_path), expected_no_path);
}

/// 迴歸測試：取自正式機實際外洩的那一行日誌。
///
/// 當時 `url=` 欄位有遮蔽，但同一筆的 `error=` 欄位（reqwest::Error 的 Debug）
/// 把含完整 Token 的 URL 又原樣印了一次，Token 因此落在 Pi 的日誌檔裡。
#[test]
fn test_redact_secrets_masks_token_inside_reqwest_error_debug() {
    let leaked = r#"reqwest::Error { kind: Request, url: "https://api.telegram.org/bot1234567890:AAFakeTokenForTestsOnlyNotARealSecret/sendMessage", source: hyper_util::client::legacy::Error(Connect, TimedOut) }"#;

    let safe = redact_secrets(leaked);

    assert!(
        !safe.contains("1234567890:AAFakeTokenForTestsOnlyNotARealSecret"),
        "Token 不得出現在遮蔽後的字串中：{safe}"
    );
    assert!(safe.contains("/bot<redacted>/sendMessage"));
    // 其餘診斷資訊必須完整保留，否則日誌就失去除錯價值。
    assert!(safe.contains("kind: Request"));
    assert!(safe.contains("TimedOut"));
}

/// 同一段文字出現多次 Token 時，每一處都要遮掉。
///
/// 這是不能直接沿用「只處理第一個 /bot」那套邏輯的理由：錯誤訊息常常
/// 同時含有外層描述與內層 source 的 URL。
#[test]
fn test_redact_secrets_masks_every_occurrence() {
    let text = "failed https://api.telegram.org/bot111:AAA/sendMessage;                     retry https://api.telegram.org/bot222:BBB/sendPhoto";

    let safe = redact_secrets(text);

    assert!(!safe.contains("111:AAA"), "第一個 Token 未遮蔽：{safe}");
    assert!(!safe.contains("222:BBB"), "第二個 Token 未遮蔽：{safe}");
    assert_eq!(safe.matches("<redacted>").count(), 2);
}

/// Token 後面接的不一定是 `/`。以「後面第一個 /」判斷邊界時，
/// 這個案例會一路吃到不相干的內容或整段漏掉。
#[test]
fn test_redact_secrets_handles_non_slash_token_boundary() {
    let quoted = r#"url: "https://api.telegram.org/bot999:ZZZ", status: 404"#;

    let safe = redact_secrets(quoted);

    assert!(!safe.contains("999:ZZZ"));
    assert!(
        safe.contains(r#"/bot<redacted>", status: 404"#),
        "Token 之後的內容須原樣保留：{safe}"
    );
}

/// 路徑剛好叫 `/bot` 但後面不是 Token 的一般網站，不應被誤遮。
#[test]
fn test_redact_secrets_keeps_non_token_bot_paths() {
    let url = "https://example.com/bot/status";

    assert_eq!(redact_secrets(url), url);
}

/// 不含特徵字串的一般爬蟲網址原樣返回。
#[test]
fn test_redact_secrets_passes_through_plain_text() {
    let text = "Failed to send https://histock.tw/stock/rank.aspx?p=all after 3 retries";

    assert_eq!(redact_secrets(text), text);
}
