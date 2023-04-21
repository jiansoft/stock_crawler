use crate::internal::logging;
use anyhow::*;
use core::{result::Result::Ok, result::Result::*};
use encoding::{DecoderTrap, Encoding};
use rust_decimal::Decimal;
use std::{collections::HashSet, str::FromStr};

#[allow(dead_code)]
pub fn big5_to_utf8(text: &str) -> Option<String> {
    let text_to_char = text.chars();
    let mut vec = Vec::with_capacity(text.len());
    for c in text_to_char {
        vec.push(c as u8);
    }

    return match encoding::all::BIG5_2003.decode(&vec, DecoderTrap::Ignore) {
        Ok(big5) => {
            return match encoding::all::UTF_8.decode(big5.as_bytes(), DecoderTrap::Ignore) {
                Ok(utf8) => Some(utf8),
                Err(why) => {
                    logging::error_file_async(format!("err:{:?}", why));
                    None
                }
            };
        }
        Err(why) => {
            logging::error_file_async(format!("err:{:?}", why));
            None
        }
    };
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
