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
