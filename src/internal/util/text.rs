use anyhow::*;
use core::{result::{
    Result::Ok,
    Result::*
}};
use encoding::{DecoderTrap, Encoding};
use rust_decimal::Decimal;
use std::{collections::HashSet, str::FromStr};

#[allow(dead_code)]
pub fn big5_to_utf8(text: &str) -> Result<String> {
    let text_to_char = text.chars();
    let mut vec = Vec::with_capacity(text.len());
    for c in text_to_char {
        vec.push(c as u8);
    }

    big5_2_utf8(vec)
}

/// Converts a Big5 encoded `Vec<u8>` to a UTF-8 `String`.
///
/// This function tries to decode the input `Vec<u8>` using the BIG5_2003 encoding
/// and then re-encodes the decoded string using the UTF-8 encoding.
/// If any of the decoding steps fail, it logs the error and returns None.
///
/// # Arguments
///
/// * `data: Vec<u8>`: The input vector of bytes containing Big5 encoded text.
///
/// # Returns
///
/// * `Option<String>`: A UTF-8 encoded string if the conversion is successful, or None if an error occurs.
pub fn big5_2_utf8(data: Vec<u8>) -> Result<String> {
    match encoding::all::BIG5_2003.decode(&data, DecoderTrap::Ignore) {
        Ok(big5) => {
            match encoding::all::UTF_8.decode(big5.as_bytes(), DecoderTrap::Ignore) {
                Ok(utf8) => Ok(utf8),
                Err(why) => {
                    Err(anyhow!("Failed to UTF_8.decode because: {:?}", why))
                }
            }
        }
        Err(why) => {
            Err(anyhow!("Failed to BIG5_2003.decode because: {:?}", why))
        }
    }
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

/// 將字串轉成 Decimal
pub fn parse_decimal(s: &str) -> Result<Decimal> {
    Decimal::from_str(&s.replace(',', ""))
        .map_err(|why| anyhow!(format!("Failed to Decimal::from_str because {:?}", why)))
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
}
