//! # 官方本益比與股價淨值比
//!
//! 證交所與櫃買中心每天公布個股的本益比、股價淨值比。日線（`DailyQuotes`）裡這兩欄
//! 原本有大量缺值（2019～2021 年股價淨值比全為 0、上櫃本益比早年幾乎全為 0），
//! 這裡定義回補用的資料型別與倉儲介面。
//!
//! 預設**只填目前為 0 的欄位，不覆寫既有值**（[`PePolicy::FillZeroOnly`]）。例外是上櫃本益比：
//! 每日收盤流程取自櫃買 openapi（只給最新一天），2026-10-12 抽查 2025-06-30 的上櫃本益比
//! 有 78% 與官方相差超過 5%（如 6488 資料庫 11.80、官方 18.55），因此上櫃本益比以官方值為準
//! 直接覆寫（[`PePolicy::Authoritative`]）；上市本益比與兩市場淨值比抽查都與官方一致，維持只填 0。

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

/// 本益比的寫入規則。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PePolicy {
    /// 只填目前為 0 的本益比。
    FillZeroOnly,
    /// 以官方值為準：與官方不同就覆寫，官方沒有值（虧損、`N/A`）時寫 0。
    /// 處理的日期也改為所有交易日（錯誤的值不是 0，只看缺值日會漏掉）。
    Authoritative,
}

/// 官方倍數回補需要的倉儲操作。股價淨值比一律只填 0。
#[async_trait]
pub trait OfficialRatioRepository: Send + Sync {
    /// `from`～`to` 間指定市場（2 上市、4 上櫃）需要處理的交易日，依日期由舊到新：
    /// [`PePolicy::FillZeroOnly`] 只回仍有本益比或淨值比為 0 的日子，[`PePolicy::Authoritative`]
    /// 回所有交易日。只看有成交（收盤價 > 0）的普通股與特別股，不含 ETF（`00` 開頭）。
    async fn fetch_dates_to_update(
        &self,
        market_id: i32,
        from: NaiveDate,
        to: NaiveDate,
        policy: PePolicy,
    ) -> Result<Vec<NaiveDate>>;

    /// 把 `ratios` 寫入 `date` 當天的日線，本益比依 `policy`、淨值比只填 0，回傳更新列數。
    async fn fill_official_ratios(
        &self,
        date: NaiveDate,
        ratios: &[OfficialRatio],
        policy: PePolicy,
    ) -> Result<u64>;
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
