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
mod tests {
    use std::time::Instant;

    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    /// 驗證 MarkdownV2 跳脫規則：保留字元前補反斜線、一般字元原樣保留。
    ///
    /// 測試案例刻意混合「需跳脫」（`_`、`*`、`[`、`]`、`(`、`)`）與
    /// 「不需跳脫」（英數字）的字元，確認只有保留字元被加上 `\`。
    #[test]
    fn test_escape_markdown_v2() {
        let input = "Hello_World*Test[link](url)";
        let expected = "Hello\\_World\\*Test\\[link\\]\\(url\\)";
        assert_eq!(escape_markdown_v2(input), expected);

        // 不含任何保留字元的字串應原封不動。
        assert_eq!(escape_markdown_v2("台積電2330"), "台積電2330");

        // 空字串是合法輸入，輸出也應該是空字串。
        assert_eq!(escape_markdown_v2(""), "");
    }

    /// 驗證 Big5 轉 UTF-8。
    #[test]
    fn test_big5_to_utf8() {
        //let wording = "¹A·~¬ì§Þ·~";
        let wording = "¦³»ùÃÒ¨é¥N¸¹¤Î¦WºÙ";
        let utf8_wording = big5_to_utf8(wording).unwrap();

        println!("big5 :{} {:?}", wording, wording.as_bytes());

        println!("utf8 :{} {:?}", utf8_wording, utf8_wording.as_bytes());
        assert_eq!(utf8_wording, "有價證券代號及名稱");
    }

    /// 驗證 Big5 bytes 可正確轉成 UTF-8 中文字串。
    #[test]
    fn test_big5_2_utf8() {
        let data = [
            0xa6, 0xb3, 0xbb, 0xf9, 0xc3, 0xd2, 0xa8, 0xe9, 0xa5, 0x4e, 0xb8, 0xb9, 0xa4, 0xce,
            0xa6, 0x57, 0xba, 0xd9,
        ];

        let actual = big5_2_utf8(&data).unwrap();

        assert_eq!(actual, "有價證券代號及名稱");
    }

    /// 驗證中文字拆字結果。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_split() {
        dotenvy::dotenv().ok();
        let chinese_word = "台積電";
        let start = Instant::now();
        let result = split(chinese_word);
        let end = start.elapsed();
        println!("split: {:?}, elapsed time: {:?}", result, end);
        assert_eq!(result, vec!["台", "台積", "台積電", "積", "積電", "電"]);
    }

    /// 比較兩種拆字實作的結果與耗時。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_split_all() {
        dotenvy::dotenv().ok();
        let _result = split_v1("2330台積電2330");
        let _result = split("2330台積電2330");

        let start = Instant::now();
        let result = split_v1("2330台積電2330");
        let duration = start.elapsed();
        println!("split_v1() result: {:?}, duration: {:?}", result, duration);

        let start = Instant::now();
        let result = split("2330台積電2330");
        let duration = start.elapsed();
        println!("split   () result: {:?}, duration: {:?}", result, duration);
    }

    /*    #[tokio::test]
    async fn test_big5_to_utf8_() {
        let wording = "¹A·~¬ì§Þ·~";
        let utf8_wording = big5_to_utf8_(wording).await.unwrap();
        println!("big5 :{} {:?}", wording, wording.as_bytes());
        println!("utf8 :{} {:?}", utf8_wording, utf8_wording.as_bytes());
    }*/

    /// 驗證跳脫字元清理結果。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_clean_string_escape_chars() {
        dotenvy::dotenv().ok();
        let chinese_word = "台積電% 元 ,";
        let start = Instant::now();
        let result = clean_escape_chars(chinese_word, Some(vec!['元', '%', '%', ',']));
        let end = start.elapsed();
        println!(
            "clean_string_escape_chars: {:?}, elapsed time: {:?}",
            result, end
        );
        assert_eq!(result, "台積電");
    }

    #[test]
    fn truncate_respects_utf8_boundaries_and_marks_truncation() {
        assert_eq!(truncate("台積電", 2), "台積...");
        assert_eq!(truncate("台積電", 3), "台積電");
        assert_eq!(truncate("abc", 0), "...");
    }

    #[test]
    fn split_removes_known_noise_and_deduplicates_sorted_tokens() {
        let result = split("A-A*");

        assert_eq!(result, vec!["A", "AA"]);
        assert_eq!(result, split_v1("A-A*"));
    }

    #[test]
    fn parse_number_helpers_remove_default_and_custom_escape_chars() {
        assert_eq!(
            parse_decimal("1,234.50元", None).unwrap(),
            Decimal::from_str("1234.50").unwrap()
        );
        assert_eq!(parse_f64("+1,234.5%", None).unwrap(), 1234.5);
        assert_eq!(parse_i32("(1,234)", Some(vec!['(', ')'])).unwrap(), 1234);
        assert_eq!(parse_i64("\"9,876\"\n", None).unwrap(), 9876);
    }

    #[test]
    fn parse_number_helpers_report_cleaned_value_on_error() {
        let decimal_err = parse_decimal("N/A元", None).unwrap_err().to_string();
        let f64_err = parse_f64("--", Some(vec!['-'])).unwrap_err().to_string();
        let i32_err = parse_i32("abc", None).unwrap_err().to_string();
        let i64_err = parse_i64("abc", None).unwrap_err().to_string();

        assert!(decimal_err.contains("Failed to parse 'N/A' as Decimal"));
        assert!(f64_err.contains("Failed to parse '' as f64"));
        assert!(i32_err.contains("Failed to parse 'abc' as i32"));
        assert!(i64_err.contains("Failed to parse 'abc' as i64"));
    }
}
