use chrono::{DateTime, Local, NaiveDate};
use rust_decimal::Decimal;

/// 股息發放日程之領域實體 (Aggregate Root)。
///
/// 封裝單一股票在特定發放年度/季度的股利結構（現金、股票股利），
/// 並定義判定持股是否具備領取資格的商業規則。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dividend {
    /// 序號
    pub serial: i64,
    /// 發放年度
    pub year: i32,
    /// 股利所屬年度
    pub year_of_dividend: i32,
    /// 發放季度
    pub quarter: String,
    /// 股票代號
    pub security_code: String,
    /// 盈餘現金股利
    pub earnings_cash_dividend: Decimal,
    /// 公積現金股利
    pub capital_reserve_cash_dividend: Decimal,
    /// 現金股利合計
    pub cash_dividend: Decimal,
    /// 盈餘股票股利
    pub earnings_stock_dividend: Decimal,
    /// 公積股票股利
    pub capital_reserve_stock_dividend: Decimal,
    /// 股票股利合計
    pub stock_dividend: Decimal,
    /// 合計股利(元)
    pub sum: Decimal,
    /// 盈餘分配率_配息(%)
    pub payout_ratio_cash: Decimal,
    /// 盈餘分配率_配股(%)
    pub payout_ratio_stock: Decimal,
    /// 盈餘分配率(%)
    pub payout_ratio: Decimal,
    /// 除息日
    pub ex_dividend_date_cash: String,
    /// 除權日
    pub ex_dividend_date_stock: String,
    /// 現金股利發放日
    pub payable_date_cash: String,
    /// 股票股利發放日
    pub payable_date_stock: String,
    /// 建立時間
    pub created_time: DateTime<Local>,
    /// 最後更新時間
    pub updated_time: DateTime<Local>,
}

impl Dividend {
    /// 判斷持有日是否符合除息日資格。
    ///
    /// 規則：持有日必須嚴格早於除息日，才能領取現金股利。
    pub fn is_eligible_for_cash(&self, holding_date: NaiveDate) -> bool {
        self.is_eligible_for_date(holding_date, &self.ex_dividend_date_cash)
    }

    /// 判斷持有日是否符合除權日資格。
    ///
    /// 規則：持有日必須嚴格早於除權日，才能領取股票股利。
    pub fn is_eligible_for_stock(&self, holding_date: NaiveDate) -> bool {
        self.is_eligible_for_date(holding_date, &self.ex_dividend_date_stock)
    }

    /// 核心判定輔助方法。
    ///
    /// 比對持有日與公告的除權息日。若日期無效、未公佈或格式不合，回傳不可領取。
    fn is_eligible_for_date(&self, holding_date: NaiveDate, ex_date_str: &str) -> bool {
        let Ok(ex_date) = NaiveDate::parse_from_str(ex_date_str, "%Y-%m-%d") else {
            return false;
        };
        holding_date < ex_date
    }

    /// 依據持有日與持股數量，計算實際可領取的股利金額與股數。
    ///
    /// 回傳格式為：`(現金股利元, 股票股利股數, 股票股利元, 合計股利元)`。
    /// 每股配股 1 元等同於配發 0.1 股 (配股率 = 股利 / 10)。
    pub fn calculate_payout(
        &self,
        holding_date: NaiveDate,
        share_quantity: Decimal,
    ) -> (Decimal, Decimal, Decimal, Decimal) {
        use rust_decimal_macros::dec;

        // 1. 現金股利計算：持有日早於除息日則依每股股利乘以持股數
        let cash = if self.is_eligible_for_cash(holding_date) {
            self.cash_dividend * share_quantity
        } else {
            Decimal::ZERO
        };

        // 2. 股票股利面值計算：持有日早於除權日
        let stock_money = if self.is_eligible_for_stock(holding_date) {
            self.stock_dividend * share_quantity
        } else {
            Decimal::ZERO
        };

        // 3. 換算實際配發的股息股數（面額為 10 元）
        let stock = stock_money / dec!(10);

        (cash, stock, stock_money, cash + stock_money)
    }
}

/// 指定日期有除權或除息事件的股票資料領域實體。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockDividendInfo {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 股票產業分類編號。
    pub stock_industry_id: i32,
    /// 現金股利（元）。
    pub cash_dividend: Decimal,
    /// 股票股利（股）。
    pub stock_dividend: Decimal,
    /// 股利合計（元）。
    pub sum: Decimal,
    /// 參考收盤價。
    pub closing_price: Decimal,
    /// 總殖利率（%）。
    pub dividend_yield: Decimal,
    /// 現金殖利率（%）。
    pub cash_dividend_yield: Decimal,
    /// 是否於查詢日期進行除息。
    pub is_cash_ex_dividend_on_date: bool,
    /// 是否於查詢日期進行除權。
    pub is_stock_ex_dividend_on_date: bool,
    /// 股利期別（`M01`～`M12` 月配、`Q1`～`Q4` 季配、`H1`／`H2` 半年配，其餘為年配）。
    pub quarter: String,
}

impl StockDividendInfo {
    /// 一年配息次數與名稱：依期別判斷，`M` 月配 12 次、`Q` 季配 4 次、`H` 半年配 2 次；
    /// 其餘（年度、`A`、空白）視為年配，回傳 `None`。
    pub fn payout_frequency(&self) -> Option<(u32, &'static str)> {
        let mut chars = self.quarter.chars();
        let kind = chars.next()?;
        if !chars.as_str().chars().all(|c| c.is_ascii_digit()) || chars.as_str().is_empty() {
            return None;
        }
        match kind {
            'M' => Some((12, "月配")),
            'Q' => Some((4, "季配")),
            'H' => Some((2, "半年配")),
            _ => None,
        }
    }

    /// 年化殖利率（%）：這次的股利合計 ÷ 參考收盤價 × 一年配息次數，四捨五入到小數兩位。
    ///
    /// 只有一年多次配息時才有意義；年配或沒有參考價時回傳 `None`。
    /// 用未四捨五入的股利與價格計算，避免單次殖利率的捨入誤差被放大 12 倍。
    pub fn annualized_yield(&self) -> Option<Decimal> {
        let (times, _) = self.payout_frequency()?;
        if self.closing_price <= Decimal::ZERO {
            return None;
        }
        Some(
            (self.sum / self.closing_price * Decimal::ONE_HUNDRED * Decimal::from(times))
                .round_dp(2),
        )
    }

    /// 查詢日期當天實際生效的（現金股利, 股票股利）。
    ///
    /// 除息日與除權日可能不同天，只有當天生效的那一項才會影響當天的參考價。
    pub fn effective_on_date(&self) -> (Decimal, Decimal) {
        let cash = if self.is_cash_ex_dividend_on_date {
            self.cash_dividend
        } else {
            Decimal::ZERO
        };
        let stock = if self.is_stock_ex_dividend_on_date {
            self.stock_dividend
        } else {
            Decimal::ZERO
        };
        (cash, stock)
    }
}

/// 計算除權息參考價：`(前日收盤 − 現金股利) ÷ (1 + 股票股利 ÷ 面額)`，四捨五入到小數兩位。
///
/// `stock_dividend` 為每股配發的股票股利（元），除以面額 10 元即配股率。
/// 前日收盤不為正、當天沒有任何除權息，或算出的參考價不為正時回傳 `None`。
///
/// 交易所的參考價另有升降單位的進位規則，這裡的值只用於「成交價是否偏離合理區間」的檢查，
/// 小數兩位的精度已足夠。
pub fn ex_rights_reference_price(
    previous_close: Decimal,
    cash_dividend: Decimal,
    stock_dividend: Decimal,
) -> Option<Decimal> {
    if previous_close <= Decimal::ZERO
        || (cash_dividend <= Decimal::ZERO && stock_dividend <= Decimal::ZERO)
    {
        return None;
    }

    let par_value = Decimal::from(crate::domain::performance::PAR_VALUE);
    let reference = (previous_close - cash_dividend) / (Decimal::ONE + stock_dividend / par_value);

    (reference > Decimal::ZERO).then(|| reference.round_dp(2))
}

/// 股票除息的發放日程資料領域實體。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockDividendPayableDateInfo {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 現金股利（元）。
    pub cash_dividend: Decimal,
    /// 股票股利（股）。
    pub stock_dividend: Decimal,
    /// 股利合計（元）。
    pub sum: Decimal,
    /// 現金股利發放日。
    pub payable_date1: String,
    /// 股票股利發放日。
    pub payable_date2: String,
    /// 除息日。
    pub ex_dividend_date1: String,
    /// 除權日。
    pub ex_dividend_date2: String,
}

impl crate::core::util::map::Keyable for Dividend {
    fn key(&self) -> String {
        format!(
            "{}-{}-{}",
            self.security_code, self.year_of_dividend, self.quarter
        )
    }

    fn key_with_prefix(&self) -> String {
        format!(
            "Dividend:{}-{}-{}",
            self.security_code, self.year_of_dividend, self.quarter
        )
    }
}

#[cfg(test)]
mod ex_rights_reference_price_tests {
    use super::*;
    use rust_decimal_macros::dec;

    /// 1235 興泰 2026-09-24 同日除權息：現金 0.5、股票 0.5 元，前日收盤 41.15。
    #[test]
    fn combines_cash_and_stock_dividend() {
        assert_eq!(
            ex_rights_reference_price(dec!(41.15), dec!(0.5), dec!(0.5)),
            Some(dec!(38.71))
        );
    }

    /// 2542 興富發 2026-09-23 除息 4 元：前日收盤 45.45。
    #[test]
    fn cash_only() {
        assert_eq!(
            ex_rights_reference_price(dec!(45.45), dec!(4), Decimal::ZERO),
            Some(dec!(41.45))
        );
    }

    #[test]
    fn stock_only() {
        assert_eq!(
            ex_rights_reference_price(dec!(110), Decimal::ZERO, dec!(1)),
            Some(dec!(100))
        );
    }

    #[test]
    fn none_without_dividend_or_close() {
        assert_eq!(
            ex_rights_reference_price(dec!(41.15), Decimal::ZERO, Decimal::ZERO),
            None
        );
        assert_eq!(
            ex_rights_reference_price(Decimal::ZERO, dec!(0.5), Decimal::ZERO),
            None
        );
        assert_eq!(
            ex_rights_reference_price(dec!(1), dec!(2), Decimal::ZERO),
            None
        );
    }

    fn info(cash_today: bool, stock_today: bool) -> StockDividendInfo {
        StockDividendInfo {
            stock_symbol: "1235".to_string(),
            name: "興泰".to_string(),
            stock_industry_id: 1,
            cash_dividend: dec!(0.5),
            stock_dividend: dec!(0.3),
            sum: dec!(0.8),
            closing_price: dec!(41.15),
            dividend_yield: Decimal::ZERO,
            cash_dividend_yield: Decimal::ZERO,
            is_cash_ex_dividend_on_date: cash_today,
            is_stock_ex_dividend_on_date: stock_today,
            quarter: String::new(),
        }
    }

    /// 期別決定一年配息次數；年配與無法辨識的期別不算多次配息。
    #[test]
    fn payout_frequency_follows_the_period_label() {
        let with = |quarter: &str| StockDividendInfo {
            quarter: quarter.to_string(),
            ..info(true, false)
        };
        assert_eq!(with("M07").payout_frequency(), Some((12, "月配")));
        assert_eq!(with("Q4").payout_frequency(), Some((4, "季配")));
        assert_eq!(with("H1").payout_frequency(), Some((2, "半年配")));
        for quarter in ["", "A", "M", "Qx", "X1"] {
            assert_eq!(with(quarter).payout_frequency(), None, "{quarter:?}");
        }
    }

    /// 年化殖利率用未捨入的股利與價格乘上配息次數（00953B 月配 0.067 元、9.49 元 → 8.47%）。
    #[test]
    fn annualized_yield_multiplies_by_payouts_per_year() {
        let monthly = StockDividendInfo {
            cash_dividend: dec!(0.067),
            stock_dividend: Decimal::ZERO,
            sum: dec!(0.067),
            closing_price: dec!(9.49),
            quarter: "M09".to_string(),
            ..info(true, false)
        };
        assert_eq!(monthly.annualized_yield(), Some(dec!(8.47)));

        let quarterly = StockDividendInfo {
            sum: dec!(0.5),
            closing_price: dec!(20),
            quarter: "Q3".to_string(),
            ..info(true, false)
        };
        assert_eq!(quarterly.annualized_yield(), Some(dec!(10)));

        assert_eq!(info(true, false).annualized_yield(), None, "年配");
        let no_price = StockDividendInfo {
            closing_price: Decimal::ZERO,
            ..monthly
        };
        assert_eq!(no_price.annualized_yield(), None);
    }

    /// 除息、除權不同天時，只計入當天生效的那一項。
    #[test]
    fn effective_on_date_keeps_only_today_events() {
        assert_eq!(info(true, true).effective_on_date(), (dec!(0.5), dec!(0.3)));
        assert_eq!(
            info(true, false).effective_on_date(),
            (dec!(0.5), Decimal::ZERO)
        );
        assert_eq!(
            info(false, true).effective_on_date(),
            (Decimal::ZERO, dec!(0.3))
        );
    }
}
