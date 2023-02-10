use crate::logging;
use encoding::{DecoderTrap, Encoding};
use once_cell::sync::Lazy;
use reqwest::{Client, IntoUrl};

pub mod cache_share;
mod crawler;
mod database;
mod free_dns;
pub mod scheduler;

static CLIENT: Lazy<Client> = Lazy::new(Default::default);

///
pub async fn request_get<T: IntoUrl>(url: T) -> Option<String> {
    let res = CLIENT.get(url).send().await;
    match res {
        Ok(res) => {
            return match res.text().await {
                Ok(t) => Some(t),
                Err(why) => {
                    logging::error_file_async(format!("{:?}", why));
                    None
                }
            }
        }
        Err(why) => {
            logging::error_file_async(format!("{:?}", why));
            None
        }
    }
}

pub async fn request_get_big5<T: IntoUrl>(url: T) -> Option<String> {
    let res = CLIENT.get(url).send().await;
    match res {
        Ok(res) => {
            return match res.text_with_charset("Big5").await {
                Ok(t) => Some(t),
                Err(why) => {
                    logging::error_file_async(format!("{:?}", why));
                    None
                }
            }
        }
        Err(why) => {
            logging::error_file_async(format!("{:?}", why));
            None
        }
    }
}

pub fn big5_to_utf8(text: &str) -> Option<String> {
    //println!("text {:?}", text.as_bytes());
    let text_to_char = text.chars();
    let mut vec = Vec::new();
    for c in text_to_char {
        //print!(" {:?}",c as u32);
        let rune = c as u8;
        vec.push(rune);
    }
    //println!("vec {:?}", vec.by_ref());
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

/*pub async fn big5_to_utf8_(text: &str) -> Result<String, Cow<'static, str>> {
    let text_to_char = text.chars();
    let mut vec = Vec::new();
    for c in text_to_char {
        let rune = c as u8;
        vec.push(rune);
    }

    return match encoding::all::BIG5_2003.decode(&*vec, DecoderTrap::Ignore) {
        Ok(big5) => {
            return encoding::all::UTF_8.decode(big5.as_bytes(), DecoderTrap::Ignore);
        }
        Err(why) => Err(why),
    };
}

*/

#[cfg(test)]
mod tests {
    // 注意這個慣用法：在 tests 模組中，從外部範疇匯入所有名字。
    use super::*;

    #[test]
     fn test_big5_to_utf8() {
        //let wording = "¹A·~¬ì§Þ·~";
        let wording ="¦³»ùÃÒ¨é¥N¸¹¤Î¦WºÙ";
        let utf8_wording = big5_to_utf8(wording).unwrap();
        println!("big5 :{} {:?}", wording, wording.as_bytes());
        println!("utf8 :{} {:?}", utf8_wording, utf8_wording.as_bytes());
    }

    /*    #[tokio::test]
    async fn test_big5_to_utf8_() {
        let wording = "¹A·~¬ì§Þ·~";
        let utf8_wording = big5_to_utf8_(wording).await.unwrap();
        println!("big5 :{} {:?}", wording, wording.as_bytes());
        println!("utf8 :{} {:?}", utf8_wording, utf8_wording.as_bytes());
    }*/
}
