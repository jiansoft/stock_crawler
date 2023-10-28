use std::{collections::HashSet, str::FromStr};

use anyhow::*;
use encoding::{DecoderTrap, Encoding};
use rust_decimal::Decimal;

const NUMBER_ESCAPE_CHAR: &[char] = &['元', '%', ',', ' ', '"', '\n'];

#[allow(dead_code)]
pub fn big5_to_utf8(text: &str) -> Result<String> {
    let text_to_char = text.chars();
    let mut vec = Vec::with_capacity(text.len());
    for c in text_to_char {
        vec.push(c as u8);
    }

    big5_2_utf8(vec.as_ref())
}

/// Converts a Big5 encoded `Vec<u8>` to a UTF-8 `String`.
///
/// This function tries to decode the input `Vec<u8>` using the BIG5_2003 encoding
/// and then re-encodes the decoded string using the UTF-8 encoding.
/// If any of the decoding steps fail, it generates an error and returns it wrapped in `Result`.
///
/// # Arguments
///
/// * `data: &[u8]`: The input vector of bytes containing Big5 encoded text.
///
/// # Returns
///
/// * `Result<String>`: A UTF-8 encoded string if the conversion is successful, or an error if the conversion fails.
pub fn big5_2_utf8(data: &[u8]) -> Result<String> {
    let big5 = encoding::all::BIG5_2003
        .decode(data, DecoderTrap::Ignore)
        .map_err(|why| anyhow!(format!("Failed to BIG5_2003.decode because {:?}", why)))?;

    encoding::all::UTF_8
        .decode(big5.as_bytes(), DecoderTrap::Ignore)
        .map_err(|why| anyhow!(format!("Failed to UTF_8.decode because {:?}", why)))
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
            if words.iter().any(|x| *x == w) {
                continue;
            }
            words.push(w);
        }
    }

    words.sort();
    words
}

#[allow(dead_code)]
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
///         that may include commas as thousands separators and other escape characters.
/// * `escape_chars`: Optional characters to be escaped from the input string.
///
/// # Returns
///
/// * `Result<Decimal>`: The parsed `Decimal` value if successful, or an error
///                      if the conversion fails.
///
/// # Example
///
/// ```
/// let s = "1,234.56";
/// let decimal_value = parse_decimal(s, Some(vec![','])).unwrap();
/// ```
pub fn parse_decimal(s: &str, escape_chars: Option<Vec<char>>) -> Result<Decimal> {
    let cleaned = clean_escape_chars(s, escape_chars);
    Decimal::from_str(&cleaned)
        .map_err(|why| anyhow!("Failed to parse '{}' as Decimal because {:?}", cleaned, why))
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
///         that may include commas as thousands separators.
///
/// * `escape_chars`: A list of additional characters to be removed from the
///                   string before parsing.
///
/// # Returns
///
/// * `Result<i32>`: The parsed `i32` value if successful, or an error
///                  if the conversion fails.
///
/// # Example
///
/// ```
/// let s = "1,234";
/// let i32_value = parse_i32(s, None).unwrap();
/// ```
pub fn parse_i32(s: &str, escape_chars: Option<Vec<char>>) -> Result<i32> {
    let cleaned = clean_escape_chars(s, escape_chars);
    i32::from_str(&cleaned)
        .map_err(|why| anyhow!("Failed to parse '{}' as i32 because: {:?}", cleaned, why))
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
///         that may include commas as thousands separators.
///
/// * `escape_chars`: A list of additional characters to be removed from the
///                   string before parsing.
///
/// # Returns
///
/// * `Result<i64>`: The parsed `i64` value if successful, or an error
///                  if the conversion fails.
///
/// # Example
///
/// ```
/// let s = "1,234";
/// let i64_value = parse_i32(s, None).unwrap();
/// ```
pub fn parse_i64(s: &str, escape_chars: Option<Vec<char>>) -> Result<i64> {
    let cleaned = clean_escape_chars(s, escape_chars);
    i64::from_str(&cleaned)
        .map_err(|why| anyhow!("Failed to parse '{}' as i64 because: {:?}", cleaned, why))
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
///                   string if found.
///
/// # Returns
///
/// * `String`: The cleaned string without any of the specified escape
///             characters.
///
/// # Example
///
/// ```
/// let s = "Hello$Wor^ld!@#";
/// let escape_chars = Some(vec!['$', '^', '@', '#']);
/// let clean_s = clean_string_escape_chars(s, escape_chars);
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

    #[test]
    fn test_big5_to_utf8() {
        //let wording = "¹A·~¬ì§Þ·~";
        let wording = "¦³»ùÃÒ¨é¥N¸¹¤Î¦WºÙ";
        let utf8_wording = big5_to_utf8(wording).unwrap();

        println!("big5 :{} {:?}", wording, wording.as_bytes());

        println!("utf8 :{} {:?}", utf8_wording, utf8_wording.as_bytes());
    }

    #[tokio::test]
    async fn test_split() {
        dotenv::dotenv().ok();
        let chinese_word = "台積電";
        let start = Instant::now();
        let result = split(chinese_word);
        let end = start.elapsed();
        println!("split: {:?}, elapsed time: {:?}", result, end);
    }

    #[tokio::test]
    async fn test_split_all() {
        dotenv::dotenv().ok();
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

    #[tokio::test]
    async fn test_clean_string_escape_chars() {
        dotenv::dotenv().ok();
        let chinese_word = "台積電% 元 ,";
        let start = Instant::now();
        let result = clean_escape_chars(chinese_word, Some(vec!['元', '%', '%', ',']));
        let end = start.elapsed();
        println!(
            "clean_string_escape_chars: {:?}, elapsed time: {:?}",
            result, end
        );
    }
}
