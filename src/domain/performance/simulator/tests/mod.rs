//! 績效模擬器的測試。
//!
//! 子模組：`dividends`（股利）、`corporate_actions`（分割與減資）、`edge_cases`（邊界與無效輸入）；
//! 這裡放共用的日期、事件與案例建構函式。

mod corporate_actions;
mod dividends;
mod edge_cases;

use super::*;
use chrono::Duration;
use rust_decimal_macros::dec;

/// 建立測試用日期。
pub(super) fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
}

/// 建立除權息事件；日期為 `None` 表示該項不存在。
pub(super) fn event(
    cash_date: Option<NaiveDate>,
    cash: Decimal,
    stock_date: Option<NaiveDate>,
    stock: Decimal,
) -> DividendEvent {
    DividendEvent {
        stock_symbol: "2330".to_string(),
        ex_dividend_date_cash: cash_date,
        ex_dividend_date_stock: stock_date,
        cash_dividend: cash,
        stock_dividend: stock,
    }
}

/// 建立模擬輸入的輔助結構（避免每個測試重複填欄位）。
pub(super) struct Case<'a> {
    pub(super) base_date: NaiveDate,
    pub(super) end_date: NaiveDate,
    pub(super) base_price: Decimal,
    pub(super) end_price: Decimal,
    pub(super) events: &'a [DividendEvent],
    /// 期間內的公司行動；多數案例為空。
    pub(super) corporate_actions: &'a [CorporateAction],
}

impl Case<'_> {
    /// 一年期、無除權息也無公司行動的基準情境。
    pub(super) fn plain(base_price: Decimal, end_price: Decimal) -> Self {
        Case {
            base_date: date(2020, 1, 2),
            end_date: date(2021, 1, 2),
            base_price,
            end_price,
            events: &[],
            corporate_actions: &[],
        }
    }
}

/// 建立一筆公司行動。
pub(super) fn corporate_action(effective_date: NaiveDate, share_ratio: Decimal) -> CorporateAction {
    CorporateAction {
        action_type: crate::domain::performance::CorporateActionType::Split,
        stock_symbol: "0050".to_string(),
        effective_date,
        share_ratio,
        note: String::new(),
    }
}

/// 以「再投入永遠查得到固定價格」的方式執行模擬。
pub(super) fn run_with_price(
    case: &Case<'_>,
    reinvest_price: Option<Decimal>,
) -> Option<SimulationResult> {
    let lookup = move |_: NaiveDate| reinvest_price;
    let input = SimulationInput {
        principal: dec!(10000),
        base_date: case.base_date,
        end_date: case.end_date,
        base_price: case.base_price,
        end_price: case.end_price,
        events: case.events,
        corporate_actions: case.corporate_actions,
        reinvest_prices: &lookup,
    };
    simulate(&input)
}
