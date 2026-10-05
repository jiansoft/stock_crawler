//! 日誌與錯誤訊息的敏感資訊遮罩（Telegram Bot Token）。

/// 關於 URL 敏感資訊過濾
///
/// 去除 URL 中的敏感資訊（例如 Telegram Bot Token）以避免洩露在日誌中。
///
/// ### 背景原因：
/// Telegram API URL 的路徑結構為 `https://api.telegram.org/bot<TOKEN>/sendMessage`。
/// 如果直接輸出原始 URL，任何能讀取日誌（Stdout、檔案、或外部 Seq 日誌分析工具）的人，
/// 都能輕易獲取此 Token，進而控制機器人發送惡意訊息。
///
/// ### 實作原理（安全且無 Regex 效能開銷）：
/// 1. 尋找 URL 中是否包含 Telegram 的 `/bot` 字眼。
/// 2. 若有，則利用 ASCII 的 `/bot` 長度為 4 字節特性進行切片取得 Token 開始位置，這可確保不會發生非 ASCII 字元切片導致 Rust 崩潰（panic）的問題。
/// 3. 在 Token 開始位置後方尋找第一個 `/`（代表 Token 結束與下一個 API 方法開始）。
/// 4. 將該 Token 部分替換成 `<redacted>` 遮罩後重組 URL。
/// 5. 若找不到後續斜線，代表整個 URL 在 Token 處就結束了，直接附加 `<redacted>` 後返回。
///
/// # 參數
/// * `url` - 原始的 URL 字串。
///
/// # 回傳值
/// 返回脫敏（隱藏 Token）後的安全 URL 字串。
pub fn redact_url(url: &str) -> String {
    redact_secrets(url)
}

/// Telegram Bot Token 的合法字元。
///
/// Token 形如 `<數字>:<英數與 _ - 組成的字串>`（例如
/// `1234567890:AAFakeTokenForTestsOnlyNotARealSecret`），
/// 由數字、英文字母、`:`、`_`、`-` 組成。凡是不屬於這個集合的字元
/// （`/`、`"`、`,`、空白…）都代表 Token 已經結束。
fn is_telegram_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-')
}

/// 從**任意文字**中遮蔽 Telegram Bot Token。
///
/// 與只處理單一 URL 的情境不同，這個函式是為了「URL 被夾在其他內容裡」而寫的，
/// 最典型的就是 [`reqwest::Error`] 的 `Debug` 輸出：
///
/// ```text
/// reqwest::Error { kind: Request, url: "https://api.telegram.org/bot<TOKEN>/sendMessage", source: ... }
/// ```
///
/// 這正是實際發生過的洩漏——記錄時 `url=` 欄位有遮蔽，但同一筆日誌的
/// `error=` 欄位把含完整 Token 的 URL 又原樣印了一次。
///
/// 兩個刻意的設計：
///
/// 1. **遮蔽所有出現位置**，不是只有第一個。一段錯誤訊息可能同時包含
///    外層描述與內層 `source` 的 URL，只遮第一個等於沒遮。
/// 2. **以字元類別判定 Token 邊界**，不是找「後面第一個 `/`」。在任意文字中，
///    Token 後面不一定緊接著路徑分隔符號（可能是 `"`、`,` 或空白），
///    用 `/` 判斷會一路吃到後面不相干的內容。
///
/// `/bot` 後面若沒有接任何 Token 字元（例如某網站的路徑就叫 `/bot/status`），
/// 代表那不是 Telegram Token，原樣保留。
pub fn redact_secrets(text: &str) -> String {
    const MARKER: &str = "/bot";
    const REDACTED: &str = "<redacted>";

    if !text.contains(MARKER) {
        // 不含特徵字串的一般網址或訊息（例如證交所 API），不做任何處理。
        return text.to_string();
    }

    let mut result = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(pos) = rest.find(MARKER) {
        // MARKER 全為 ASCII，pos + MARKER.len() 必定落在合法的 UTF-8 邊界上。
        let split = pos + MARKER.len();
        let after = &rest[split..];
        // find 回傳的是位元組位移；即使終止字元是多位元組字元，該位移仍是合法邊界。
        let token_len = after
            .find(|c: char| !is_telegram_token_char(c))
            .unwrap_or(after.len());

        result.push_str(&rest[..split]);

        if token_len == 0 {
            // "/bot" 後面不是 Token，繼續往後找下一個出現位置。
            rest = after;
            continue;
        }

        result.push_str(REDACTED);
        rest = &after[token_len..];
    }

    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
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
}
