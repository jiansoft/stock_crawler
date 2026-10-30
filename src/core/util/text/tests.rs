//! 文字解析工具的單元測試。

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
        0xa6, 0xb3, 0xbb, 0xf9, 0xc3, 0xd2, 0xa8, 0xe9, 0xa5, 0x4e, 0xb8, 0xb9, 0xa4, 0xce, 0xa6,
        0x57, 0xba, 0xd9,
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
fn log_snippet_flattens_whitespace_before_truncating() {
    assert_eq!(
        log_snippet("<!DOCTYPE html>\n<html lang=\"en-us\">\n  <head>", 200),
        "<!DOCTYPE html> <html lang=\"en-us\"> <head>"
    );
    assert_eq!(log_snippet("  台積電\r\n\t 2330  ", 6), "台積電 23...");
    assert_eq!(log_snippet("", 10), "");
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
