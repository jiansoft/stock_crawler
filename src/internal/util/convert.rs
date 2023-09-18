use rust_decimal::Decimal;

use crate::internal::{logging, util::text};

// Define trait that provides necessary methods
pub trait FromValue {
    fn get_string(&self, escape_chars: Option<Vec<char>>) -> String;
    fn get_i64(&self, escape_chars: Option<Vec<char>>) -> i64;
    fn get_decimal(&self, escape_chars: Option<Vec<char>>) -> Decimal;
}

/// 為  serde_json::Value 實作指定型態的轉換
impl FromValue for serde_json::Value {
    fn get_string(&self, escape_chars: Option<Vec<char>>) -> String {
        match self.as_str() {
            None => Default::default(),
            Some(v) => text::clean_escape_chars(v, escape_chars),
        }
    }
    fn get_i64(&self, escape_chars: Option<Vec<char>>) -> i64 {
        match self.as_str() {
            None => Default::default(),
            Some(v) => text::parse_i64(v, escape_chars).unwrap_or_else(|why| {
                logging::warn_file_async(format!("{:?}", why));
                Default::default()
            }),
        }
    }

    fn get_decimal(&self, escape_chars: Option<Vec<char>>) -> Decimal {
        text::parse_decimal(&self.to_string(), escape_chars).unwrap_or_else(|why| {
            logging::warn_file_async(format!("{:?}", why));
            Default::default()
        })
    }
}

/// 為  String 實作指定型態的轉換
impl FromValue for String {
    fn get_string(&self, escape_chars: Option<Vec<char>>) -> String {
        text::clean_escape_chars(self, escape_chars)
    }

    fn get_i64(&self, escape_chars: Option<Vec<char>>) -> i64 {
        text::parse_i64(self, escape_chars).unwrap_or_else(|why| {
            logging::warn_file_async(format!("{:?}", why));
            Default::default()
        })
    }

    fn get_decimal(&self, escape_chars: Option<Vec<char>>) -> Decimal {
        text::parse_decimal(self, escape_chars).unwrap_or_else(|why| {
            logging::warn_file_async(format!("{:?}", why));
            Default::default()
        })
    }
}
