use crate::logging;
use encoding::{DecoderTrap, Encoding};

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

#[cfg(test)]
mod tests {
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

    /*    #[tokio::test]
    async fn test_big5_to_utf8_() {
        let wording = "¹A·~¬ì§Þ·~";
        let utf8_wording = big5_to_utf8_(wording).await.unwrap();
        println!("big5 :{} {:?}", wording, wording.as_bytes());
        println!("utf8 :{} {:?}", utf8_wording, utf8_wording.as_bytes());
    }*/
}
