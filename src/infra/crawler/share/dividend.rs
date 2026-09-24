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

/// 依預告表的類別與配股率，判定這次事件實際的 `(是否除息, 是否除權)`。
///
/// 預告表的「權」同時涵蓋**無償配股**與**現金增資認購**（例如 2026-09-22 金山電 8042
/// 的「除權」是現增，完全沒有股利）。現增不是股利，不能讓它把事件標成除權：
///
/// - 純現增（2890、8042 這類）：類別只有「權」，會被判成兩者皆非，由呼叫端丟棄，
///   否則會被當成找不到期別的股利事件，還白白逐檔去問 Yahoo。
/// - 現金股利＋現增（3260 這類「除權息」）：只保留除息，否則會在只有現金股利的
///   資料列上寫入除權日。
///
/// 判定規則：類別含「權」、無償配股率為未公布或 0、且現增配股率大於 0 時，視為現增而非配股。
/// 無償配股率未公布但沒有現增時仍保留除權，交給後續流程等金額公布。
pub fn classify_ex_dividend(
    kind: &str,
    stock_dividend_ratio: Option<Decimal>,
    subscription_ratio: Option<Decimal>,
) -> (bool, bool) {
    let (is_cash, is_stock) = parse_ex_dividend_kind(kind);
    let no_bonus_shares = stock_dividend_ratio.is_none_or(|ratio| ratio.is_zero());
    let has_rights_issue = subscription_ratio.is_some_and(|ratio| ratio > Decimal::ZERO);

    (is_cash, is_stock && !(no_bonus_shares && has_rights_issue))
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    /// 純現增除權（2890 永豐金、8042 金山電）不是股利事件。
    #[test]
    fn classify_ex_dividend_treats_pure_rights_issue_as_non_dividend() {
        // TWSE：無償配股率空白
        assert_eq!(
            classify_ex_dividend("權", None, Some(dec!(0.04329540))),
            (false, false)
        );
        // TPEx：無償配股率為 0
        assert_eq!(
            classify_ex_dividend("除權", Some(dec!(0)), Some(dec!(0.06723675))),
            (false, false)
        );
    }

    /// 現金股利＋現增（3260 威剛）只算除息。
    #[test]
    fn classify_ex_dividend_drops_stock_flag_for_cash_plus_rights_issue() {
        assert_eq!(
            classify_ex_dividend("除權息", Some(dec!(0)), Some(dec!(0.06163263))),
            (true, false)
        );
    }

    /// 真正的配股（含同時有現增的 2614）與未公布配股率的除權都要保留。
    #[test]
    fn classify_ex_dividend_keeps_bonus_share_events() {
        assert_eq!(
            classify_ex_dividend("權息", Some(dec!(0.08)), Some(dec!(0.38195352))),
            (true, true)
        );
        assert_eq!(
            classify_ex_dividend("權息", Some(dec!(0.08)), None),
            (true, true)
        );
        assert_eq!(classify_ex_dividend("權", None, None), (false, true));
        assert_eq!(
            classify_ex_dividend("息", None, Some(dec!(0))),
            (true, false)
        );
    }
}
