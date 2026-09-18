//! # 除權除息公告共用型別
//!
//! 定義 TWSE 與 TPEx 除權除息預告共用的資料結構，
//! 以及交易所「權/息」類別字串的共用解析規則。

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::core::declare::StockExchangeMarket;

/// 除權除息預告事件。
///
/// TWSE（`TWT48U_ALL`）與 TPEx（`tpex_exright_prepost`）兩支 OpenAPI 的欄位語意一致，
/// 因此兩邊的採集器都轉譯成這個共用型別，讓上層流程不必分辨資料來自哪個市場。
///
/// # 欄位語意
///
/// - `cash_dividend` / `stock_dividend_ratio` 為 `None` 時代表「尚未公布」而**不是** 0。
///   ETF 常見的「待公告實際收益分配金額」在 OpenAPI 版會是空字串，必須與真正配發 0 元區分，
///   否則會把未公布的金額當成 0 寫進資料庫。
/// - `stock_dividend_ratio` 是**無償配股率**（股/股），不含現金增資認購配股率；
///   要換算成「元」的股票股利請用 [`ExDividendAnnouncement::stock_dividend`]。
#[derive(Debug, Clone, PartialEq)]
pub struct ExDividendAnnouncement {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 除權除息交易日。
    pub ex_date: NaiveDate,
    /// 是否為除息（配發現金股利）。
    pub is_cash: bool,
    /// 是否為除權（配發股票股利）。
    pub is_stock: bool,
    /// 現金股利（元/股）；`None` 代表尚未公布。
    pub cash_dividend: Option<Decimal>,
    /// 無償配股率（股/股）；`None` 代表尚未公布。
    pub stock_dividend_ratio: Option<Decimal>,
    /// 交易市場。
    pub market: StockExchangeMarket,
}

impl ExDividendAnnouncement {
    /// 將無償配股率換算成以「元」計價的股票股利。
    ///
    /// 台股面額固定 10 元，配股率 `0.06`（每股配 0.06 股）等於股票股利 `0.6` 元。
    pub fn stock_dividend(&self) -> Option<Decimal> {
        self.stock_dividend_ratio.map(|ratio| ratio * Decimal::TEN)
    }
}

/// 解析交易所公告的「權/息」類別字串，回傳 `(是否除息, 是否除權)`。
///
/// TWSE 用 `息`、`權`、`權息`，TPEx 用 `除息`、`除權`、`除權息`，
/// 兩者都只要判斷字串中是否出現「息」與「權」即可，不必逐一列舉。
pub fn parse_ex_dividend_kind(kind: &str) -> (bool, bool) {
    (kind.contains('息'), kind.contains('權'))
}
