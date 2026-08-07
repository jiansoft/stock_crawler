use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::domain::performance::entity::{DividendEvent, SimulationOutcome};

/// 模擬所需的輸入。
///
/// 全部為值型別，不含任何 I/O —— 這讓整段報酬邏輯可以在沒有資料庫的情況下
/// 被完整單元測試。本功能的正確性幾乎全繫於此。
#[derive(Clone)]
pub struct SimulationInput<'a> {
    /// 期初投入金額（元）。
    pub principal: Decimal,
    /// 期初交易日。
    pub base_date: NaiveDate,
    /// 期末交易日。
    pub end_date: NaiveDate,
    /// 期初收盤價。必須大於零。
    pub base_price: Decimal,
    /// 期末收盤價。必須大於零。
    pub end_price: Decimal,
    /// 期間內的除權息事件。呼叫端不需預先排序。
    pub events: &'a [DividendEvent],
    /// 各除息日的收盤價，供「含息再投入」口徑買回股數之用。
    ///
    /// 查無該日價格時該次股利改為現金累積，不強制再投入。
    pub reinvest_prices: &'a dyn Fn(NaiveDate) -> Option<Decimal>,
}

/// 三種口徑的模擬結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimulationResult {
    /// 口徑 A：純價格報酬。
    pub price: SimulationOutcome,
    /// 口徑 B：含息不再投入。
    pub total: SimulationOutcome,
    /// 口徑 C：含息再投入。
    pub reinvested: SimulationOutcome,
    /// 實際年數。
    pub years: Decimal,
    /// 期間內實際採計的除權息次數。
    pub dividend_events: i32,
}

/// 依固定投入金額模擬三種口徑的期末價值與報酬率。
///
/// # 計算規則
///
/// - 期初以 `principal / base_price` 買入（允許小數股，純模擬不取整）。
/// - 除權息事件<strong>必須依日期排序後逐筆套用</strong>：先配股再配息時，
///   配息的基數是配股後的股數，無視時序一次加總會低估現金股利。
/// - 配股率 = 每股股票股利 / 面額 10 元。
/// - 事件生效條件為除權息日落在 `(base_date, end_date]` 區間內。
///
/// # Errors
///
/// `base_price` 或 `end_price` 非正數、或 `end_date <= base_date` 時回傳 `None`。
pub fn simulate(input: &SimulationInput<'_>) -> Option<SimulationResult> {
    // TODO(M1)：實作於 Agent A。
    let _ = input;
    unimplemented!("simulate 尚未實作")
}

/// 以期末價值與年數換算年化報酬率（%）。
///
/// `rust_decimal` 沒有通用實數次方運算，故此處刻意於最後一步轉為 `f64`
/// 執行 `powf`，再轉回 `Decimal` 保留 4 位小數。金額與股數的累加全程維持
/// `Decimal`，避免逐次運算累積浮點誤差 —— 只有這一步例外。
pub fn annualized_return_pct(
    principal: Decimal,
    end_value: Decimal,
    years: Decimal,
) -> Option<Decimal> {
    // TODO(M1)：實作於 Agent A。
    let _ = (principal, end_value, years);
    unimplemented!("annualized_return_pct 尚未實作")
}

/// 以期末價值換算區間總報酬率（%）。
pub fn total_return_pct(principal: Decimal, end_value: Decimal) -> Option<Decimal> {
    // TODO(M1)：實作於 Agent A。
    let _ = (principal, end_value);
    unimplemented!("total_return_pct 尚未實作")
}
