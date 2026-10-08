//! CAGR 計算的單筆結果組裝，以及依股票代號分組的輔助函式。

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::domain::performance::{
    CagrPeriod, CorporateAction, DividendEvent, StockCagr,
    entity::PRINCIPAL,
    simulator::{SimulationInput, simulate},
};

/// 組出單一股票在單一期間的計算結果。
///
/// 資料不足時回傳的 `StockCagr` 各數值欄位為 `None`，`data_complete` 為
/// `false` —— 刻意不寫 0，否則「上市半年的新股」會在十年排行榜上被誤讀成
/// 「十年零報酬」。這類列仍然寫入資料庫，讓「某檔在某期間的狀態」永遠查得到，
/// API 才能穩定回傳、前端也不必區分「查無資料」與「不可計算」兩套邏輯。
#[allow(clippy::too_many_arguments)]
pub(super) fn build_record(
    symbol: &str,
    period: CagrPeriod,
    end_date: NaiveDate,
    target: NaiveDate,
    aligned_base_date: NaiveDate,
    base_price_on_aligned: Option<Decimal>,
    grace_quote: Option<(NaiveDate, Decimal)>,
    end_price: Option<Decimal>,
    first_quote_date: Option<NaiveDate>,
    events: &[DividendEvent],
    corporate_actions: &[CorporateAction],
    reinvest_prices: &HashMap<NaiveDate, Decimal>,
    has_anomaly: bool,
) -> StockCagr {
    let incomplete = |base_date: Option<NaiveDate>, base_price: Option<Decimal>| StockCagr {
        date: end_date,
        stock_symbol: symbol.to_string(),
        period,
        base_date,
        base_price,
        end_price,
        years: None,
        price: None,
        total: None,
        reinvested: None,
        first_quote_date,
        shortfall_days: None,
        data_complete: false,
        has_anomaly,
        dividend_events: 0,
    };

    // 期初價：優先用統一對齊日當天的報價；當天沒有才套用寬限規則。
    let (base_date, base_price) = match base_price_on_aligned {
        Some(price) => (aligned_base_date, price),
        None => match grace_quote {
            Some((quote_date, price)) => (quote_date, price),
            None => return incomplete(None, None),
        },
    };

    let Some(end_price) = end_price else {
        // 期末無報價（停牌中或當日未交易），無法結算。
        return incomplete(Some(base_date), Some(base_price));
    };

    let lookup = |date: NaiveDate| reinvest_prices.get(&date).copied();
    let input = SimulationInput {
        principal: Decimal::from(PRINCIPAL),
        base_date,
        end_date,
        base_price,
        end_price,
        events,
        corporate_actions,
        reinvest_prices: &lookup,
    };

    let Some(result) = simulate(&input) else {
        return incomplete(Some(base_date), Some(base_price));
    };

    StockCagr {
        date: end_date,
        stock_symbol: symbol.to_string(),
        period,
        base_date: Some(base_date),
        base_price: Some(base_price),
        end_price: Some(end_price),
        years: Some(result.years),
        // 近十年每年皆有 134–216 檔股票配股，期間愈長，忽略配股造成的低估
        // 愈嚴重，故長期間不提供純價格口徑。
        price: period.supports_price_metric().then_some(result.price),
        total: Some(result.total),
        reinvested: Some(result.reinvested),
        first_quote_date,
        shortfall_days: Some((base_date - target).num_days().max(0) as i32),
        data_complete: true,
        has_anomaly,
        dividend_events: result.dividend_events,
    }
}

/// 將公司行動依股票代號分組。
pub(super) fn group_corporate_actions_by_symbol(
    actions: Vec<CorporateAction>,
) -> HashMap<String, Vec<CorporateAction>> {
    let mut grouped: HashMap<String, Vec<CorporateAction>> = HashMap::new();
    for action in actions {
        grouped
            .entry(action.stock_symbol.clone())
            .or_default()
            .push(action);
    }
    grouped
}

/// 將股利事件依股票代號分組。
pub(super) fn group_events_by_symbol(
    events: Vec<DividendEvent>,
) -> HashMap<String, Vec<DividendEvent>> {
    let mut grouped: HashMap<String, Vec<DividendEvent>> = HashMap::new();
    for event in events {
        grouped
            .entry(event.stock_symbol.clone())
            .or_default()
            .push(event);
    }
    grouped
}

/// 收集「含息再投入」口徑需要查價的 (股票代號, 除息日) 組合。
///
/// 只收集區間內、且確實有現金股利的事件 —— 配股不需要查價，區間外的事件
/// 也不會生效。這讓查詢量從「所有股票 × 所有交易日」縮到實際需要的數萬筆。
pub(super) fn collect_reinvest_pairs(
    events_by_symbol: &HashMap<String, Vec<DividendEvent>>,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<(String, NaiveDate)> {
    let mut pairs = Vec::new();
    let mut seen: HashSet<(String, NaiveDate)> = HashSet::new();
    for events in events_by_symbol.values() {
        for event in events {
            let Some(date) = event.ex_dividend_date_cash else {
                continue;
            };
            if event.cash_dividend <= Decimal::ZERO || date <= from || date > to {
                continue;
            }
            let key = (event.stock_symbol.clone(), date);
            if seen.insert(key.clone()) {
                pairs.push(key);
            }
        }
    }
    pairs
}

/// 將 (代號, 日期, 價格) 攤平結果整理成以代號為索引的查價表。
pub(super) fn group_prices_by_symbol(
    prices: Vec<(String, NaiveDate, Decimal)>,
) -> HashMap<String, HashMap<NaiveDate, Decimal>> {
    let mut grouped: HashMap<String, HashMap<NaiveDate, Decimal>> = HashMap::new();
    for (symbol, date, price) in prices {
        grouped.entry(symbol).or_default().insert(date, price);
    }
    grouped
}
