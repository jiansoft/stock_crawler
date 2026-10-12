//! # 官方本益比與股價淨值比
//!
//! 證交所與櫃買中心每天公布個股的本益比、股價淨值比。日線（`DailyQuotes`）裡這兩欄
//! 原本有大量缺值（2019～2021 年股價淨值比全為 0、上櫃本益比早年幾乎全為 0），
//! 這裡定義回補用的資料型別與倉儲介面：**只填目前為 0 的欄位，不覆寫既有值**。

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use rust_decimal::Decimal;

/// 單一股票某日的官方本益比與股價淨值比。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficialRatio {
    /// 股票代號。
    pub stock_symbol: String,
    /// 本益比；虧損或交易所未提供（`-`、`N/A`、`0.00`）時為 `None`。
    pub price_earning_ratio: Option<Decimal>,
    /// 股價淨值比；未提供時為 `None`。
    pub price_to_book_ratio: Option<Decimal>,
}

impl OfficialRatio {
    /// 至少有一個可用的倍數。
    pub fn has_any(&self) -> bool {
        self.price_earning_ratio.is_some() || self.price_to_book_ratio.is_some()
    }
}

/// 官方倍數回補需要的倉儲操作。
#[async_trait]
pub trait OfficialRatioRepository: Send + Sync {
    /// `from`～`to` 間，指定市場（2 上市、4 上櫃）仍有本益比或股價淨值比為 0 的交易日，
    /// 依日期由舊到新。只看有成交（收盤價 > 0）的普通股與特別股，不含 ETF（`00` 開頭）。
    async fn fetch_dates_missing_ratios(
        &self,
        market_id: i32,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<NaiveDate>>;

    /// 把 `ratios` 填入 `date` 當天的日線：只更新目前為 0 且官方有值的欄位，回傳更新列數。
    async fn fill_official_ratios(&self, date: NaiveDate, ratios: &[OfficialRatio]) -> Result<u64>;
}

/// 解析交易所的倍數字串：去掉千分位逗號與空白，`-`、`N/A`、空字串與非正數視為沒有值。
pub fn parse_ratio(raw: &str) -> Option<Decimal> {
    let cleaned = raw.trim().replace(',', "");
    cleaned
        .parse::<Decimal>()
        .ok()
        .filter(|value| *value > Decimal::ZERO)
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    /// 正數照常解析（含千分位）；`-`、`N/A`、空字串、0 與負數都是沒有值。
    #[test]
    fn parse_ratio_handles_placeholders() {
        assert_eq!(parse_ratio(" 13.82 "), Some(dec!(13.82)));
        assert_eq!(parse_ratio("1,234.50"), Some(dec!(1234.50)));
        for raw in ["-", "N/A", "", "0.00", "-3.2"] {
            assert_eq!(parse_ratio(raw), None, "{raw}");
        }
    }

    /// 兩個倍數都沒有時 has_any 為假。
    #[test]
    fn has_any_requires_one_ratio() {
        let mut ratio = OfficialRatio {
            stock_symbol: "1101".to_owned(),
            price_earning_ratio: None,
            price_to_book_ratio: None,
        };
        assert!(!ratio.has_any());
        ratio.price_to_book_ratio = Some(dec!(1.2));
        assert!(ratio.has_any());
    }
}
