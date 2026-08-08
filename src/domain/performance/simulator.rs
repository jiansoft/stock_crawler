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
mod tests {
    use super::*;
    use chrono::Duration;
    use rust_decimal_macros::dec;

    /// 建立測試用日期。
    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
    }

    /// 建立除權息事件；日期為 `None` 表示該項不存在。
    fn event(
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
    struct Case<'a> {
        base_date: NaiveDate,
        end_date: NaiveDate,
        base_price: Decimal,
        end_price: Decimal,
        events: &'a [DividendEvent],
        /// 期間內的公司行動；多數案例為空。
        corporate_actions: &'a [CorporateAction],
    }

    impl Case<'_> {
        /// 一年期、無除權息也無公司行動的基準情境。
        fn plain(base_price: Decimal, end_price: Decimal) -> Self {
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
    fn corporate_action(effective_date: NaiveDate, share_ratio: Decimal) -> CorporateAction {
        CorporateAction {
            stock_symbol: "0050".to_string(),
            effective_date,
            share_ratio,
            note: String::new(),
        }
    }

    /// 以「再投入永遠查得到固定價格」的方式執行模擬。
    fn run_with_price(
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

    /// 1. 完全沒有股利時，三種口徑的結果必須完全一致。
    #[test]
    fn test_no_dividend_all_metrics_equal() {
        let case = Case {
            base_date: date(2020, 1, 2),
            end_date: date(2021, 1, 2),
            base_price: dec!(100),
            end_price: dec!(150),
            events: &[],
            corporate_actions: &[],
        };
        let result = run_with_price(&case, Some(dec!(120))).expect("應可計算");

        // 期初買入 100 股，期末 150 元 → 15,000 元。
        assert_eq!(result.price.end_shares, dec!(100));
        assert_eq!(result.price.end_value, dec!(15000));
        assert_eq!(result.price, result.total);
        assert_eq!(result.total, result.reinvested);
        assert_eq!(result.dividend_events, 0);
        assert_eq!(result.price.total_return_pct, dec!(50));
    }

    /// 2. 僅有現金股利：口徑 B 的現金累積正確，且口徑 A 的期末價值低於 B。
    #[test]
    fn test_cash_dividend_only() {
        let events = [event(Some(date(2020, 7, 1)), dec!(5), None, Decimal::ZERO)];
        let case = Case {
            base_date: date(2020, 1, 2),
            end_date: date(2021, 1, 2),
            base_price: dec!(100),
            end_price: dec!(100),
            events: &events,
            corporate_actions: &[],
        };
        // 再投入查價回傳 None，讓口徑 C 也走現金累積，方便與 B 對照。
        let result = run_with_price(&case, None).expect("應可計算");

        // 100 股 × 5 元 = 500 元現金。
        assert_eq!(result.total.cash_received, dec!(500));
        assert_eq!(result.total.end_shares, dec!(100));
        assert_eq!(result.total.end_value, dec!(10500));
        // 口徑 A 忽略股利，期末價值必然較低。
        assert_eq!(result.price.cash_received, Decimal::ZERO);
        assert_eq!(result.price.end_value, dec!(10000));
        assert!(result.price.end_value < result.total.end_value);
        assert_eq!(result.dividend_events, 1);
    }

    /// 3. 僅有股票股利：股數增加為「原股數 ×(1 + 股票股利 / 10)」。
    #[test]
    fn test_stock_dividend_only() {
        let events = [event(None, Decimal::ZERO, Some(date(2020, 7, 1)), dec!(1))];
        let case = Case {
            base_date: date(2020, 1, 2),
            end_date: date(2021, 1, 2),
            base_price: dec!(100),
            end_price: dec!(100),
            events: &events,
            corporate_actions: &[],
        };
        let result = run_with_price(&case, Some(dec!(90))).expect("應可計算");

        // 100 股配股 1 元（配股率 0.1）→ 110 股。
        assert_eq!(result.total.end_shares, dec!(110));
        assert_eq!(result.reinvested.end_shares, dec!(110));
        // 口徑 A 不理會配股，股數維持 100。
        assert_eq!(result.price.end_shares, dec!(100));
        assert!(result.price.end_shares < result.total.end_shares);
        assert_eq!(result.total.cash_received, Decimal::ZERO);
        assert_eq!(result.dividend_events, 1);
    }

    /// 4. 先配股、後配息：配息基數必須是「配股後」的股數。
    ///
    /// 若錯誤地把所有配息一次加總（以期初股數為基數），會得到 200 元現金；
    /// 正確依時序套用應為 110 股 × 2 元 = 220 元。
    #[test]
    fn test_stock_then_cash_uses_post_split_shares() {
        let events = [event(
            Some(date(2020, 8, 1)),
            dec!(2),
            Some(date(2020, 7, 1)),
            dec!(1),
        )];
        let case = Case {
            base_date: date(2020, 1, 2),
            end_date: date(2021, 1, 2),
            base_price: dec!(100),
            end_price: dec!(100),
            events: &events,
            corporate_actions: &[],
        };
        let result = run_with_price(&case, None).expect("應可計算");

        assert_eq!(result.total.end_shares, dec!(110));
        // 關鍵斷言：220 而非 200。
        assert_eq!(result.total.cash_received, dec!(220));
        assert_eq!(result.total.end_value, dec!(11220));
        // 同一筆事件雖產生兩個動作，僅計 1 次。
        assert_eq!(result.dividend_events, 1);
    }

    /// 5. 先配息、後配股：與第 4 題對照，配息基數為期初股數。
    #[test]
    fn test_cash_then_stock_uses_pre_split_shares() {
        let events = [event(
            Some(date(2020, 7, 1)),
            dec!(2),
            Some(date(2020, 8, 1)),
            dec!(1),
        )];
        let case = Case {
            base_date: date(2020, 1, 2),
            end_date: date(2021, 1, 2),
            base_price: dec!(100),
            end_price: dec!(100),
            events: &events,
            corporate_actions: &[],
        };
        let result = run_with_price(&case, None).expect("應可計算");

        assert_eq!(result.total.end_shares, dec!(110));
        // 配息在配股之前 → 100 股 × 2 元 = 200 元。
        assert_eq!(result.total.cash_received, dec!(200));
        assert_eq!(result.total.end_value, dec!(11200));
        assert_eq!(result.dividend_events, 1);
    }

    /// 6. 同一日同時除息與除權：約定先除息、後除權。
    #[test]
    fn test_same_day_cash_before_stock() {
        let same_day = date(2020, 7, 1);
        let events = [event(Some(same_day), dec!(2), Some(same_day), dec!(1))];
        let case = Case {
            base_date: date(2020, 1, 2),
            end_date: date(2021, 1, 2),
            base_price: dec!(100),
            end_price: dec!(100),
            events: &events,
            corporate_actions: &[],
        };
        let result = run_with_price(&case, None).expect("應可計算");

        // 先除息（100 股 × 2 = 200）再除權（→ 110 股）。
        assert_eq!(result.total.cash_received, dec!(200));
        assert_eq!(result.total.end_shares, dec!(110));
    }

    /// 6b. 多筆事件、現金與股票除權息日交錯，驗證混合排序而非分兩迴圈。
    #[test]
    fn test_interleaved_dates_across_events() {
        // 事件 A：除權 2020-03-01（配股 1 元）；除息 2020-09-01（現金 1 元）
        // 事件 B：除息 2020-06-01（現金 1 元）；除權 2020-12-01（配股 1 元）
        let events = [
            event(
                Some(date(2020, 9, 1)),
                dec!(1),
                Some(date(2020, 3, 1)),
                dec!(1),
            ),
            event(
                Some(date(2020, 6, 1)),
                dec!(1),
                Some(date(2020, 12, 1)),
                dec!(1),
            ),
        ];
        let case = Case {
            base_date: date(2020, 1, 2),
            end_date: date(2021, 1, 2),
            base_price: dec!(100),
            end_price: dec!(100),
            events: &events,
            corporate_actions: &[],
        };
        let result = run_with_price(&case, None).expect("應可計算");

        // 正確時序：03/01 配股 →110 股；06/01 配息 110 元；
        // 09/01 配息 110 元；12/01 配股 →121 股。
        assert_eq!(result.total.end_shares, dec!(121));
        assert_eq!(result.total.cash_received, dec!(220));
        assert_eq!(result.dividend_events, 2);
    }

    /// 7. 區間外的事件必須被排除：早於期初、等於期初、晚於期末皆不採計。
    #[test]
    fn test_events_outside_range_excluded() {
        let base = date(2020, 1, 2);
        let end = date(2021, 1, 2);
        let events = [
            // 早於期初。
            event(Some(date(2019, 12, 1)), dec!(5), None, Decimal::ZERO),
            // 等於期初（開區間，不採計）。
            event(Some(base), dec!(5), None, Decimal::ZERO),
            // 晚於期末。
            event(Some(date(2021, 2, 1)), dec!(5), None, Decimal::ZERO),
            // 等於期末（閉區間，採計）。
            event(Some(end), dec!(3), None, Decimal::ZERO),
        ];
        let case = Case {
            base_date: base,
            end_date: end,
            base_price: dec!(100),
            end_price: dec!(100),
            events: &events,
            corporate_actions: &[],
        };
        let result = run_with_price(&case, None).expect("應可計算");

        // 只有期末當日那筆生效：100 股 × 3 元 = 300 元。
        assert_eq!(result.total.cash_received, dec!(300));
        assert_eq!(result.dividend_events, 1);
    }

    /// 分割：三個口徑的持股都按比例換算，報酬率不受影響。
    ///
    /// 這正是 0050 在 2025-06-18 的情形——期初 188.65、期末 47.57 看似
    /// 大跌，實際上是 1 股換 4 股，持股價值幾乎不變。
    #[test]
    fn test_split_scales_shares_in_every_metric() {
        let actions = [corporate_action(date(2020, 6, 18), dec!(4))];
        let case = Case {
            corporate_actions: &actions,
            ..Case::plain(dec!(200), dec!(50))
        };

        let result = run_with_price(&case, None).expect("應可計算");

        // 期初 10000/200 = 50 股，分割後 200 股，期末 200 × 50 = 10000。
        assert_eq!(result.price.end_shares, dec!(200));
        assert_eq!(result.price.end_value, dec!(10000));
        assert_eq!(result.price.total_return_pct, Decimal::ZERO);
        assert_eq!(result.total.end_shares, dec!(200));
        assert_eq!(result.reinvested.end_shares, dec!(200));
        // 分割不是除權息，不列入事件數。
        assert_eq!(result.dividend_events, 0);
    }

    /// 沒有登錄分割時，同一情境會被算成 −75% —— 這正是要修正的錯誤。
    #[test]
    fn test_without_the_split_the_return_is_catastrophically_wrong() {
        let result = run_with_price(&Case::plain(dec!(200), dec!(50)), None).expect("應可計算");

        assert_eq!(result.price.end_shares, dec!(50));
        assert_eq!(result.price.end_value, dec!(2500));
        assert_eq!(result.price.total_return_pct, dec!(-75));
    }

    /// 減資：比例小於 1，股數等比例縮減。
    #[test]
    fn test_capital_reduction_shrinks_shares() {
        // 減資三成：1,000 股變 700 股，參考價相應上調。
        let actions = [corporate_action(date(2020, 6, 18), dec!(0.7))];
        let case = Case {
            corporate_actions: &actions,
            ..Case::plain(dec!(70), dec!(100))
        };

        let result = run_with_price(&case, None).expect("應可計算");

        // 期初 10000/70 股 × 0.7 = 100 股，期末 100 × 100 = 10000。
        assert_eq!(result.price.end_shares, dec!(100));
        assert_eq!(result.price.end_value, dec!(10000));
        assert_eq!(result.price.total_return_pct, Decimal::ZERO);
    }

    /// 區間外的公司行動不得生效，期初日當天生效者也不算。
    #[test]
    fn test_corporate_actions_outside_the_window_are_ignored() {
        let actions = [
            // 期初日之前。
            corporate_action(date(2019, 6, 1), dec!(4)),
            // 期初日當天：該日報價已是調整後價格，再乘一次會重複計算。
            corporate_action(date(2020, 1, 2), dec!(4)),
            // 期末日之後。
            corporate_action(date(2021, 6, 1), dec!(4)),
        ];
        let case = Case {
            corporate_actions: &actions,
            ..Case::plain(dec!(100), dec!(100))
        };

        let result = run_with_price(&case, None).expect("應可計算");

        assert_eq!(result.price.end_shares, dec!(100));
        assert_eq!(result.price.total_return_pct, Decimal::ZERO);
    }

    /// 期末日當天生效的分割仍要採計（左開右閉）。
    #[test]
    fn test_split_on_the_end_date_is_applied() {
        let actions = [corporate_action(date(2021, 1, 2), dec!(2))];
        let case = Case {
            corporate_actions: &actions,
            ..Case::plain(dec!(100), dec!(50))
        };

        let result = run_with_price(&case, None).expect("應可計算");

        assert_eq!(result.price.end_shares, dec!(200));
        assert_eq!(result.price.total_return_pct, Decimal::ZERO);
    }

    /// 同日除息與分割：現金股利以分割前的股數計算。
    #[test]
    fn test_dividend_on_the_split_date_uses_pre_split_shares() {
        let events = [DividendEvent {
            stock_symbol: "0050".to_string(),
            ex_dividend_date_cash: Some(date(2020, 6, 18)),
            ex_dividend_date_stock: None,
            cash_dividend: dec!(2),
            stock_dividend: Decimal::ZERO,
        }];
        let actions = [corporate_action(date(2020, 6, 18), dec!(4))];
        let case = Case {
            events: &events,
            corporate_actions: &actions,
            ..Case::plain(dec!(200), dec!(50))
        };

        let result = run_with_price(&case, None).expect("應可計算");

        // 除息基數是分割前的 50 股 → 現金 100 元；若誤用分割後的 200 股
        // 會得到 400 元，把配息灌成四倍。
        assert_eq!(result.total.cash_received, dec!(100));
        assert_eq!(result.total.end_shares, dec!(200));
        assert_eq!(result.dividend_events, 1);
    }

    /// 比例為零或負數的登錄錯誤一律略過，不得讓持股歸零。
    #[test]
    fn test_non_positive_ratio_is_ignored() {
        for ratio in [Decimal::ZERO, dec!(-2)] {
            let actions = [corporate_action(date(2020, 6, 18), ratio)];
            let case = Case {
                corporate_actions: &actions,
                ..Case::plain(dec!(100), dec!(100))
            };

            let result = run_with_price(&case, None).expect("應可計算");
            assert_eq!(result.price.end_shares, dec!(100), "ratio={ratio}");
        }
    }

    /// 8. 非法輸入一律回傳 `None`。
    #[test]
    fn test_invalid_inputs_return_none() {
        let lookup = |_: NaiveDate| None;
        let base = SimulationInput {
            principal: dec!(10000),
            base_date: date(2020, 1, 2),
            end_date: date(2021, 1, 2),
            base_price: dec!(100),
            end_price: dec!(100),
            events: &[],
            corporate_actions: &[],
            reinvest_prices: &lookup,
        };

        // 期初價為零。
        let mut tmp = base.clone();
        tmp.base_price = Decimal::ZERO;
        assert!(simulate(&tmp).is_none());

        // 期末價為零。
        let mut tmp1 = base.clone();
        tmp1.end_price = Decimal::ZERO;
        assert!(simulate(&tmp1).is_none());

        // 期末日等於期初日。
        let mut tmp2 = base.clone();
        tmp2.end_date = tmp2.base_date;
        assert!(simulate(&tmp2).is_none());

        // 期末日早於期初日。
        let mut tmp3 = base.clone();
        tmp3.end_date = date(2019, 1, 2);
        assert!(simulate(&tmp3).is_none());

        // 投入金額非正數。
        let mut tmp4 = base.clone();
        tmp4.principal = Decimal::ZERO;
        assert!(simulate(&tmp4).is_none());

        // 負數價格。
        let mut tmp5 = base;
        tmp5.base_price = dec!(-1);
        assert!(simulate(&tmp5).is_none());
    }

    /// 9. 再投入口徑查無除息日價格時，該次股利退回現金累積（不可遺失）。
    #[test]
    fn test_reinvest_falls_back_to_cash_when_price_missing() {
        let with_price = date(2020, 4, 1);
        let without_price = date(2020, 10, 1);
        let events = [
            event(Some(with_price), dec!(2), None, Decimal::ZERO),
            event(Some(without_price), dec!(2), None, Decimal::ZERO),
        ];
        let lookup = move |d: NaiveDate| {
            if d == with_price {
                Some(dec!(50))
            } else {
                None
            }
        };
        let input = SimulationInput {
            principal: dec!(10000),
            base_date: date(2020, 1, 2),
            end_date: date(2021, 1, 2),
            base_price: dec!(100),
            end_price: dec!(100),
            events: &events,
            corporate_actions: &[],
            reinvest_prices: &lookup,
        };
        let result = simulate(&input).expect("應可計算");

        // 第一次：100 股 × 2 = 200 元，以 50 元買回 4 股 → 104 股。
        // 第二次：104 股 × 2 = 208 元，查無價格 → 累積現金。
        assert_eq!(result.reinvested.end_shares, dec!(104));
        assert_eq!(result.reinvested.cash_received, dec!(208));
        assert_eq!(
            result.reinvested.end_value,
            dec!(104) * dec!(100) + dec!(208)
        );
        // 口徑 B 全數累積現金：200 + 200 = 400。
        assert_eq!(result.total.cash_received, dec!(400));
        assert_eq!(result.dividend_events, 2);
    }

    /// 10. 長期間（10 年、20 次除息）驗證 Decimal 累加不失真與年化正確性。
    #[test]
    fn test_long_horizon_precision() {
        let base = date(2010, 1, 4);
        // 刻意讓日數差恰為 3650 天，使 years 正好等於 10。
        let end = base + Duration::days(3650);
        let events: Vec<DividendEvent> = (1..=20)
            .map(|i| {
                event(
                    Some(base + Duration::days(180 * i)),
                    dec!(1),
                    None,
                    Decimal::ZERO,
                )
            })
            .collect();
        let case = Case {
            base_date: base,
            end_date: end,
            base_price: dec!(100),
            end_price: dec!(200),
            events: &events,
            corporate_actions: &[],
        };
        let result = run_with_price(&case, None).expect("應可計算");

        assert_eq!(result.years, dec!(10));
        assert_eq!(result.dividend_events, 20);
        // 100 股不變（無配股），每次配息 100 元 × 20 次 = 2,000 元，且完全不失真。
        assert_eq!(result.total.end_shares, dec!(100));
        assert_eq!(result.total.cash_received, dec!(2000));
        assert_eq!(result.total.end_value, dec!(22000));
        assert_eq!(result.total.total_return_pct, dec!(120));

        // 手算年化：(22000 / 10000)^(1/10) - 1。
        let expected = ((2.2_f64).powf(0.1) - 1.0) * 100.0;
        let actual = result.total.cagr_pct.to_f64().expect("應可轉為 f64");
        assert!(
            (actual - expected).abs() < 1e-4,
            "年化報酬 {actual} 與手算 {expected} 差距過大"
        );
    }

    /// 11. `annualized_return_pct` 的邊界行為。
    #[test]
    fn test_annualized_return_pct_boundaries() {
        // years = 1 時，年化報酬等於區間總報酬。
        let cagr = annualized_return_pct(dec!(10000), dec!(12000), Decimal::ONE)
            .expect("years = 1 應可計算");
        let total = total_return_pct(dec!(10000), dec!(12000)).expect("應可計算");
        assert!(
            (cagr - total).abs() < dec!(0.0001),
            "cagr={cagr} total={total}"
        );

        // 期末價值歸零 → -100%。
        assert_eq!(
            annualized_return_pct(dec!(10000), Decimal::ZERO, dec!(5)),
            Some(dec!(-100))
        );

        // 非法輸入。
        assert!(annualized_return_pct(Decimal::ZERO, dec!(100), dec!(1)).is_none());
        assert!(annualized_return_pct(dec!(-1), dec!(100), dec!(1)).is_none());
        assert!(annualized_return_pct(dec!(10000), dec!(100), Decimal::ZERO).is_none());
        assert!(annualized_return_pct(dec!(10000), dec!(100), dec!(-1)).is_none());
        assert!(annualized_return_pct(dec!(10000), dec!(-1), dec!(1)).is_none());

        // 總報酬率的非法輸入。
        assert!(total_return_pct(Decimal::ZERO, dec!(100)).is_none());
        assert!(total_return_pct(dec!(-5), dec!(100)).is_none());
        // 虧損情境。
        assert_eq!(total_return_pct(dec!(10000), dec!(2500)), Some(dec!(-75)));
    }

    /// 12. 兩個除權息日皆為 `None`（`sort_key()` 為 `None`）的事件被安全略過。
    #[test]
    fn test_event_without_any_date_is_skipped() {
        let events = [
            event(None, dec!(5), None, dec!(1)),
            event(Some(date(2020, 7, 1)), dec!(2), None, Decimal::ZERO),
        ];
        assert!(events[0].sort_key().is_none());

        let case = Case {
            base_date: date(2020, 1, 2),
            end_date: date(2021, 1, 2),
            base_price: dec!(100),
            end_price: dec!(100),
            events: &events,
            corporate_actions: &[],
        };
        let result = run_with_price(&case, None).expect("應可計算");

        // 只有第二筆生效。
        assert_eq!(result.total.cash_received, dec!(200));
        assert_eq!(result.total.end_shares, dec!(100));
        assert_eq!(result.dividend_events, 1);
    }

    /// 13. 金額為零的除權息（日期存在但金額為 0）不應被採計。
    #[test]
    fn test_zero_amount_event_not_counted() {
        let events = [event(
            Some(date(2020, 7, 1)),
            Decimal::ZERO,
            Some(date(2020, 7, 1)),
            Decimal::ZERO,
        )];
        let case = Case {
            base_date: date(2020, 1, 2),
            end_date: date(2021, 1, 2),
            base_price: dec!(100),
            end_price: dec!(100),
            events: &events,
            corporate_actions: &[],
        };
        let result = run_with_price(&case, None).expect("應可計算");

        assert_eq!(result.dividend_events, 0);
        assert_eq!(result.total.cash_received, Decimal::ZERO);
        assert_eq!(result.total.end_shares, dec!(100));
    }
}
