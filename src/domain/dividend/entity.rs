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

/// 待計算盈餘分配率的股利列，以及對應期間的每股盈餘。
///
/// 盈餘分配率原本只能向 Goodinfo 取得，但那個站台已對機器請求全面掛上瀏覽器驗證。
/// 這個比率本來就是「配發的股利 ÷ 同期間每股盈餘」，兩項資料庫都有，因此改為自行計算。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayoutRatioCandidate {
    /// 股利列序號。
    pub serial: i64,
    /// 現金股利。
    pub cash_dividend: Decimal,
    /// 股票股利。
    pub stock_dividend: Decimal,
    /// 股利合計。
    pub sum: Decimal,
    /// 該股利所屬期間的每股盈餘；財報還沒出來時為 `None`。
    pub earnings_per_share: Option<Decimal>,
}

/// 計算完成的盈餘分配率（%）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayoutRatios {
    /// 股利列序號。
    pub serial: i64,
    /// 盈餘分配率_配息(%)。
    pub payout_ratio_cash: Decimal,
    /// 盈餘分配率_配股(%)。
    pub payout_ratio_stock: Decimal,
    /// 盈餘分配率(%)。
    pub payout_ratio: Decimal,
}

impl PayoutRatioCandidate {
    /// 依每股盈餘算出三個盈餘分配率，資料不足以計算時回傳 `None`。
    ///
    /// 每股盈餘缺漏（財報尚未公布）或不為正（虧損仍配息）時都回傳 `None`：
    /// 前者等下次排程財報到齊再算，後者的比率是負值或無限大，寫進資料庫只會污染
    /// 以分配率推估的股價區間。這兩種情況都讓該列維持 0，由畫面自行退化顯示。
    pub fn calculate(&self) -> Option<PayoutRatios> {
        let eps = self.earnings_per_share?;
        if eps <= Decimal::ZERO {
            return None;
        }

        let ratio = |dividend: Decimal| (dividend / eps * Decimal::ONE_HUNDRED).round_dp(4);

        Some(PayoutRatios {
            serial: self.serial,
            payout_ratio_cash: ratio(self.cash_dividend),
            payout_ratio_stock: ratio(self.stock_dividend),
            payout_ratio: ratio(self.sum),
        })
    }
}

#[cfg(test)]
mod payout_ratio_tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn candidate(cash: Decimal, stock: Decimal, eps: Option<Decimal>) -> PayoutRatioCandidate {
        PayoutRatioCandidate {
            serial: 1,
            cash_dividend: cash,
            stock_dividend: stock,
            sum: cash + stock,
            earnings_per_share: eps,
        }
    }

    /// 2753 的 25H2：現金 7.5、配股 0.5，對應下半年 EPS 6.58。
    #[test]
    fn calculate_splits_cash_and_stock_ratios() {
        let ratios = candidate(dec!(7.5), dec!(0.5), Some(dec!(6.58)))
            .calculate()
            .expect("expected ratios for positive eps");

        assert_eq!(ratios.payout_ratio_cash, dec!(113.9818));
        assert_eq!(ratios.payout_ratio_stock, dec!(7.5988));
        assert_eq!(ratios.payout_ratio, dec!(121.5805));
    }

    /// 財報還沒公布時維持 0，等下一輪排程重算，不要先寫一個錯的值進去。
    #[test]
    fn calculate_skips_when_eps_is_missing() {
        assert!(
            candidate(dec!(4), Decimal::ZERO, None)
                .calculate()
                .is_none()
        );
    }

    /// 虧損仍配息時分配率是負值，寫進資料庫只會污染以分配率推估的股價區間。
    #[test]
    fn calculate_skips_non_positive_eps() {
        assert!(
            candidate(dec!(4), Decimal::ZERO, Some(dec!(-1.2)))
                .calculate()
                .is_none()
        );
        assert!(
            candidate(dec!(4), Decimal::ZERO, Some(Decimal::ZERO))
                .calculate()
                .is_none()
        );
    }

    /// 只配現金時配股分配率為 0，不應該變成 NULL 或沿用合計值。
    #[test]
    fn calculate_keeps_zero_stock_ratio() {
        let ratios = candidate(dec!(8), Decimal::ZERO, Some(dec!(9.05)))
            .calculate()
            .expect("expected ratios for positive eps");

        assert_eq!(ratios.payout_ratio_stock, Decimal::ZERO);
        assert_eq!(ratios.payout_ratio_cash, ratios.payout_ratio);
    }
}
