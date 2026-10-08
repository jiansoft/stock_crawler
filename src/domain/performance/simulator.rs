use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};

use crate::domain::performance::entity::{
    CorporateAction, DividendEvent, PAR_VALUE, SimulationOutcome,
};

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
    /// 期間內的公司行動（分割／減資）。呼叫端不需預先排序也不需先過濾期間。
    ///
    /// 報價是原始成交價，這些事件會讓價格出現無法用除權息解釋的跳動；
    /// 不套用的話跨越分割日的報酬率會嚴重失真。
    pub corporate_actions: &'a [CorporateAction],
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

/// 除權息動作的種類。
///
/// 一筆 [`DividendEvent`] 的現金與股票除權息日可能不同日，因此模擬時
/// 必須先把事件「拆解」成獨立帶日期的動作，再全部混合排序後逐筆套用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ActionKind {
    /// 除息（發放現金股利）。同日時排在除權之前。
    Cash,
    /// 除權（發放股票股利）。
    Stock,
    /// 公司行動（分割／減資）造成的股數變動。
    ///
    /// 同日時排在除權息之後：現金與股票股利都以「事件前」的股數計算，
    /// 分割改變的是之後的持股基數。
    Split,
}

/// 拆解後的單一動作。
#[derive(Debug, Clone, Copy)]
struct DividendAction {
    /// 動作發生日。
    date: NaiveDate,
    /// 動作種類。
    kind: ActionKind,
    /// 每股金額（現金股利元數、股票股利元數），或分割的股數變動比例。
    amount: Decimal,
    /// 來源事件在 `events` 中的索引，用於統計採計事件筆數。
    ///
    /// 公司行動不是除權息，不列入 `dividend_events`，因此為 `None`。
    event_index: Option<usize>,
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
/// # 事件時序約定
///
/// 一筆事件的現金除息日與股票除權日可能不同日，故實作上先把每筆事件拆成
/// 最多兩個 [`DividendAction`]（除息、除權），全部混合後依「日期 → 種類」
/// 排序再逐筆套用。**同一日同時除息與除權時，約定先除息、後除權**：
/// 台股實務上現金股利是以除權息基準日的持股計算，配股在同一日入帳並不會
/// 增加當次可領的現金股利，因此除息必須先於除權生效。
///
/// # `dividend_events` 的定義
///
/// 統計的是「**至少有一個動作實際生效**（日期落在區間內且金額大於零）的
/// [`DividendEvent`] 筆數」，而非動作數。因此一筆同時除息又除權的事件
/// 只計 1 次。
///
/// # 口徑差異
///
/// - A `price`：完全忽略除權息，股數固定為 `principal / base_price`。
/// - B `total`：配股增加股數，現金股利累積為現金不再投入。
/// - C `reinvested`：配股同 B；現金股利於除息日以當日收盤價買回股數。
///   查無該日價格時該次股利退回現金累積（不可丟棄），此時 `cash_received`
///   不為零。
///
/// # Errors
///
/// `principal`、`base_price` 或 `end_price` 非正數、或 `end_date <= base_date`
/// 時回傳 `None`。
pub fn simulate(input: &SimulationInput<'_>) -> Option<SimulationResult> {
    if input.principal <= Decimal::ZERO
        || input.base_price <= Decimal::ZERO
        || input.end_price <= Decimal::ZERO
        || input.end_date <= input.base_date
    {
        return None;
    }

    // 年數一律以實際日數差 / 365 計算，避免假日對齊造成系統性偏差。
    let years =
        Decimal::from((input.end_date - input.base_date).num_days()) / Decimal::from(365_i64);
    if years <= Decimal::ZERO {
        return None;
    }

    // ── 步驟一：把事件拆解成帶日期的獨立動作 ──────────────────────────
    let par_value = Decimal::from(PAR_VALUE);
    let mut actions: Vec<DividendAction> = Vec::with_capacity(input.events.len() * 2);
    for (index, event) in input.events.iter().enumerate() {
        // 兩個日期皆為 None（sort_key() 為 None）的事件不會產生任何動作，
        // 於此自然被安全略過。
        if let Some(date) = event.ex_dividend_date_cash
            && event.cash_dividend > Decimal::ZERO
            && date > input.base_date
            && date <= input.end_date
        {
            actions.push(DividendAction {
                date,
                kind: ActionKind::Cash,
                amount: event.cash_dividend,
                event_index: Some(index),
            });
        }

        if let Some(date) = event.ex_dividend_date_stock
            && event.stock_dividend > Decimal::ZERO
            && date > input.base_date
            && date <= input.end_date
        {
            actions.push(DividendAction {
                date,
                kind: ActionKind::Stock,
                amount: event.stock_dividend,
                event_index: Some(index),
            });
        }
    }

    // 公司行動同樣拆成帶日期的動作；生效日落在 (base_date, end_date] 內才適用。
    // 期初日當天生效者不算：那天的報價已經是調整後價格，再乘一次會重複計算。
    for action in input.corporate_actions {
        if action.share_ratio > Decimal::ZERO
            && action.effective_date > input.base_date
            && action.effective_date <= input.end_date
        {
            actions.push(DividendAction {
                date: action.effective_date,
                kind: ActionKind::Split,
                amount: action.share_ratio,
                event_index: None,
            });
        }
    }

    // ── 步驟二：混合後依「日期 → 種類（除息 → 除權 → 分割）」排序 ──────
    actions.sort_by_key(|action| (action.date, action.kind));

    // ── 步驟三：逐筆套用，口徑 B 與 C 共用骨架但各自維護狀態 ───────────
    let base_shares = input.principal / input.base_price;

    // 口徑 B：含息不再投入。
    let mut total_shares = base_shares;
    let mut total_cash = Decimal::ZERO;
    // 口徑 C：含息再投入。
    let mut reinvested_shares = base_shares;
    let mut reinvested_cash = Decimal::ZERO;
    // 口徑 A：純價格。忽略除權息，但**不能**忽略分割 —— 分割不是報酬，
    // 是同一筆持股換算成不同股數，不調整等於平白虧掉四分之三。
    let mut price_shares = base_shares;

    // 採計事件的索引集合（動作數 ≠ 事件數，故需去重）。
    let mut counted_events: Vec<usize> = Vec::with_capacity(actions.len());

    for action in &actions {
        match action.kind {
            ActionKind::Cash => {
                // 除息：以「事件發生當下」的股數為基數。
                total_cash += total_shares * action.amount;

                let payout = reinvested_shares * action.amount;
                match (input.reinvest_prices)(action.date) {
                    Some(price) if price > Decimal::ZERO => {
                        reinvested_shares += payout / price;
                    }
                    // 查無當日價格（或價格非正數）時退回現金累積，不可丟棄。
                    _ => reinvested_cash += payout,
                }
            }
            ActionKind::Stock => {
                // 除權：配股率 = 每股股票股利 / 面額。
                let rate = action.amount / par_value;
                total_shares += total_shares * rate;
                reinvested_shares += reinvested_shares * rate;
            }
            ActionKind::Split => {
                // 分割／減資：三個口徑的持股都按同一比例換算。
                total_shares *= action.amount;
                reinvested_shares *= action.amount;
                price_shares *= action.amount;
            }
        }

        if let Some(index) = action.event_index
            && !counted_events.contains(&index)
        {
            counted_events.push(index);
        }
    }

    // ── 步驟四：期末結算 ────────────────────────────────────────────
    let price_outcome = build_outcome(
        input.principal,
        price_shares,
        Decimal::ZERO,
        price_shares * input.end_price,
        years,
    )?;
    let total_outcome = build_outcome(
        input.principal,
        total_shares,
        total_cash,
        total_shares * input.end_price + total_cash,
        years,
    )?;
    let reinvested_outcome = build_outcome(
        input.principal,
        reinvested_shares,
        reinvested_cash,
        reinvested_shares * input.end_price + reinvested_cash,
        years,
    )?;

    Some(SimulationResult {
        price: price_outcome,
        total: total_outcome,
        reinvested: reinvested_outcome,
        years,
        dividend_events: counted_events.len() as i32,
    })
}

/// 組裝單一口徑的結果，報酬率無法計算時回傳 `None`。
fn build_outcome(
    principal: Decimal,
    end_shares: Decimal,
    cash_received: Decimal,
    end_value: Decimal,
    years: Decimal,
) -> Option<SimulationOutcome> {
    Some(SimulationOutcome {
        end_shares,
        cash_received,
        end_value,
        total_return_pct: total_return_pct(principal, end_value)?,
        cagr_pct: annualized_return_pct(principal, end_value, years)?,
    })
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
    if principal <= Decimal::ZERO || years <= Decimal::ZERO || end_value < Decimal::ZERO {
        return None;
    }

    // 期末價值歸零＝全額虧損，年化報酬固定為 -100%（0 的任意次方仍為 0）。
    if end_value.is_zero() {
        return Some(Decimal::from(-100_i64));
    }

    let ratio = (end_value / principal).to_f64()?;
    let years = years.to_f64()?;
    if !(ratio.is_finite() && years.is_finite()) || years <= 0.0 {
        return None;
    }

    let pct = (ratio.powf(1.0 / years) - 1.0) * 100.0;
    if !pct.is_finite() {
        return None;
    }

    Some(Decimal::from_f64(pct)?.round_dp(4))
}

/// 以期末價值換算區間總報酬率（%）。
///
/// `principal` 非正數時無從定義報酬率，回傳 `None`。
pub fn total_return_pct(principal: Decimal, end_value: Decimal) -> Option<Decimal> {
    if principal <= Decimal::ZERO {
        return None;
    }

    Some(((end_value / principal - Decimal::ONE) * Decimal::from(100_i64)).round_dp(4))
}

#[cfg(test)]
mod tests;
