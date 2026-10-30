use std::{collections::HashSet, str::FromStr};

use anyhow::*;
use encoding_rs::BIG5;
use rust_decimal::Decimal;

const NUMBER_ESCAPE_CHAR: &[char] = &['元', '%', ',', ' ', '"', '\n', '+'];

#[allow(dead_code)]
/// 截斷字串並確保不破壞 UTF-8 字元邊界。
pub fn truncate(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        None => s.to_string(),
        Some((idx, _)) => format!("{}...", &s[..idx]),
    }
}

/// 把回應內容壓成適合寫進日誌的單行片段：連續空白（含換行）併成一個空格後再截斷。
///
/// 錯誤頁常是多行 HTML，直接截斷會讓一筆日誌拆成好幾行。
pub fn log_snippet(s: &str, max_chars: usize) -> String {
    truncate(
        &s.split_whitespace().collect::<Vec<_>>().join(" "),
        max_chars,
    )
}

/// 將疑似 Big5 編碼字串轉成 UTF-8。
pub fn big5_to_utf8(text: &str) -> Result<String> {
    let text_to_char = text.chars();
    let mut vec = Vec::with_capacity(text.len());
    for c in text_to_char {
        vec.push(c as u8);
    }

    big5_2_utf8(vec.as_ref())
}

/// Converts Big5 encoded bytes to a UTF-8 `String`.
///
/// This function decodes the input bytes with `encoding_rs::BIG5`. Decode errors are handled
/// by `encoding_rs` with replacement characters, matching the previous best-effort conversion
/// behavior that ignored malformed input instead of failing the whole response.
///
/// # Arguments
///
/// * `data`: The input bytes containing Big5 encoded text.
///
/// # Returns
///
/// * `Result<String>`: A UTF-8 encoded string if the conversion is successful, or an error if the conversion fails.
pub fn big5_2_utf8(data: &[u8]) -> Result<String> {
    let (decoded, _, _) = BIG5.decode(data);
    Ok(decoded.into_owned())
}

/// 跳脫 Telegram `MarkdownV2` 的保留字元。
///
/// ## 為什麼需要這個函式
///
/// 本專案的通知訊息採用 Telegram 的 `MarkdownV2` 格式（可以加粗體、程式碼區塊
/// 等排版）。這個格式規定：`_ * [ ] ( ) ~ ` > # + - = | { } . !` 這 18 個字元
/// 是「保留字元」，若要當成普通文字顯示，前面必須加上反斜線 `\` 跳脫，
/// 否則 Telegram API 會直接回 `400 Bad Request`，整則訊息發不出去。
///
/// 股票名稱、錯誤訊息這類「動態內容」隨時可能含有這些字元（例如 `台積電-KY`
/// 的 `-`、小數點 `.`），所以組訊息時所有動態內容都要先經過這個函式。
///
/// ## 為什麼放在 `core::util::text` 而不是 telegram 模組
///
/// 依 DDD 分層，`app`（use case）不應該 import `interfaces`（傳輸層）。
/// 但 app 層組通知訊息時需要這個跳脫規則——若它住在
/// `interfaces::bot::telegram`，app 就被迫反向依賴外層。
/// 因此把它下沉到 `core`（誰都可以依賴的共用工具層）：
/// app 組訊息用它、`interfaces::bot::telegram` 的 adapter 也用它，
/// 依賴方向全部合法（外層 → 內層）。
///
/// ## 逐步說明
///
/// 1. `SPECIALS` 列出 MarkdownV2 規格定義的所有保留字元。
/// 2. 預先配置 `text.len() * 2` 的字串容量——最壞情況是每個字元都要跳脫
///    （每個字元前面都插一個 `\`，長度翻倍），一次配足避免中途重新配置記憶體。
/// 3. 逐字元走訪：遇到保留字元就先補一個反斜線，然後照抄原字元。
pub fn escape_markdown_v2(text: impl Into<String>) -> String {
    // MarkdownV2 規格（https://core.telegram.org/bots/api#markdownv2-style）
    // 定義的保留字元清單。
    const SPECIALS: &[char] = &[
        '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
    ];

    let text = text.into();
    // 預留兩倍空間：跳脫最多讓長度翻倍，先配足就不會在迴圈中反覆重新分配。
    let mut result = String::with_capacity(text.len() * 2);

    for ch in text.chars() {
        if SPECIALS.contains(&ch) {
            // 保留字元：先放一個反斜線，Telegram 才會把後面的字元當普通文字。
            result.push('\\');
        }
        result.push(ch);
    }
    result
}

/// 將中文字拆分 例︰台積電 => ["台", "台積", "台積電", "積", "積電", "電"]
pub fn split(w: &str) -> Vec<String> {
    let word = w.replace(['*', '-'], "");
    let text_rune = word.chars().collect::<Vec<_>>();
    let text_len = text_rune.len();
    let mut words = Vec::with_capacity(text_len * 3);

    for i in 0..text_len {
        for ii in (i + 1)..=text_len {
            let w = text_rune[i..ii].iter().collect::<String>();
            if words.contains(&w) {
                continue;
            }
            words.push(w);
        }
    }

    words.sort();
    words
}

#[allow(dead_code)]
/// 以 `HashSet` 去重的舊版拆字實作。
pub fn split_v1(w: &str) -> Vec<String> {
    let word = w.replace(['*', '-'], "");
    let text_rune = word.chars().collect::<Vec<_>>();
    let text_len = text_rune.len();
    // let mut words = Vec::with_capacity(text_len * 3);
    let mut set = HashSet::with_capacity(text_len * 3);

    for i in 0..text_len {
        for ii in (i + 1)..=text_len {
            let w = text_rune[i..ii].iter().collect::<String>();
            if !set.contains(&w) {
                set.insert(w.clone());
                // words.push(w);
            }
        }
    }
    let mut words: Vec<String> = set.into_iter().collect();
    words.sort();
    words
}

/// Parses a decimal value from a given string.
///
/// This function accepts a string representation of a decimal number,
/// potentially containing commas as thousands separators and other escape characters,
/// and attempts to convert it into a `Decimal`. If the conversion fails, an error is returned.
///
/// # Arguments
///
/// * `s`: A string slice containing the representation of a decimal number
///   that may include commas as thousands separators and other escape characters.
/// * `escape_chars`: Optional characters to be escaped from the input string.
///
/// # Returns
///
/// * `Result<Decimal>`: The parsed `Decimal` value if successful,
///   or an error if the conversion fails.
///
/// # Example
///
/// ```
/// use stock_crawler::core::util::text::parse_decimal;
/// let s = "1,234.56";
/// let decimal_value = parse_decimal(s, Some(vec![','])).unwrap();
/// ```
pub fn parse_decimal(s: &str, escape_chars: Option<Vec<char>>) -> Result<Decimal> {
    let cleaned = clean_escape_chars(s, escape_chars);
    // 用 with_context 而不是 anyhow!("... {:?}", why)：
    // 前者把底層解析錯誤保留在 source chain（呼叫端可用 {:#} 或 source() 取得），
    // 後者只把錯誤「印成文字」，原始錯誤型別與鏈路都會遺失。
    Decimal::from_str(&cleaned).with_context(|| format!("Failed to parse '{cleaned}' as Decimal"))
}

/// 將字串解析為 `f64`。
pub fn parse_f64(s: &str, escape_chars: Option<Vec<char>>) -> Result<f64> {
    let cleaned = clean_escape_chars(s, escape_chars);
    // 同 parse_decimal：以 context 保留底層錯誤鏈。
    f64::from_str(&cleaned).with_context(|| format!("Failed to parse '{cleaned}' as f64"))
}

/// Parses an `i32` value from a given string.
///
/// This function accepts a string representation of an `i32` number,
/// potentially containing commas as thousands separators, and attempts to
/// convert it into an `i32`. If the conversion fails, an error is returned.
///
/// # Arguments
///
/// * `s`: A string slice containing the representation of an `i32` number
///   that may include commas as thousands separators.
///
/// * `escape_chars`: A list of additional characters to be removed from the
///   string before parsing.
///
/// # Returns
///
/// * `Result<i32>`: The parsed `i32` value if successful, or an error
///   if the conversion fails.
///
/// # Example
///
/// ```
/// use stock_crawler::core::util::text::parse_i32;
/// let s = "1,234";
/// let i32_value = parse_i32(s, None).unwrap();
/// ```
pub fn parse_i32(s: &str, escape_chars: Option<Vec<char>>) -> Result<i32> {
    let cleaned = clean_escape_chars(s, escape_chars);
    // 同 parse_decimal：以 context 保留底層錯誤鏈。
    i32::from_str(&cleaned).with_context(|| format!("Failed to parse '{cleaned}' as i32"))
}

/// Parses an `i64` value from a given string.
///
/// This function accepts a string representation of an `i64` number,
/// potentially containing commas as thousands separators, and attempts to
/// convert it into an `i32`. If the conversion fails, an error is returned.
///
/// # Arguments
///
/// * `s`: A string slice containing the representation of an `i64` number
///   that may include commas as thousands separators.
///
/// * `escape_chars`: A list of additional characters to be removed from the
///   string before parsing.
///
/// # Returns
///
/// * `Result<i64>`: The parsed `i64` value if successful, or an error
///   if the conversion fails.
///
/// # Example
///
/// ```
/// use stock_crawler::core::util::text::parse_i64;
/// let s = "1,234";
/// let i64_value = parse_i64(s, None).unwrap();
/// ```
pub fn parse_i64(s: &str, escape_chars: Option<Vec<char>>) -> Result<i64> {
    let cleaned = clean_escape_chars(s, escape_chars);
    // 同 parse_decimal：以 context 保留底層錯誤鏈。
    i64::from_str(&cleaned).with_context(|| format!("Failed to parse '{cleaned}' as i64"))
}

/// Removes a set of escape characters from a given string.
///
/// This function accepts a string and a list of escape characters and
/// produces a new string that doesn't contain any occurrences of these
/// characters.
///
/// # Arguments
///
/// * `s`: The original string from which escape characters will be removed.
///
/// * `escape_chars`: Optional characters that will be removed from the
///   string if found.
///
/// # Returns
///
/// * `String`: The cleaned string without any of the specified escape
///   characters.
///
/// # Example
///
/// ```ignore
/// let s = "Hello$Wor^ld!@#";
/// let escape_chars = Some(vec!['$', '^', '@', '#']);
/// let clean_s = clean_escape_chars(s, escape_chars);
/// assert_eq!(clean_s, "HelloWorld!");
/// ```
pub(crate) fn clean_escape_chars(s: &str, escape_chars: Option<Vec<char>>) -> String {
    let mut combined: Vec<char> = NUMBER_ESCAPE_CHAR.to_vec();
    if let Some(ec) = escape_chars {
        combined.extend(ec);
    }

    let filters = combined.iter().collect::<HashSet<_>>();
    s.chars().filter(|c| !filters.contains(c)).collect()
}

#[cfg(test)]
mod tests;
