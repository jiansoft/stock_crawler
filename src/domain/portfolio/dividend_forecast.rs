//! # 持股股利現金流預估
//!
//! 以目前持股股數，預估未來幾個月每月會收到多少現金股利：
//!
//! - **已公告**：除息日已公布、發放日落在預估期間內的配息；沒有發放日時以除息日後
//!   [`PAYABLE_LAG_DAYS`] 天推估。
//! - **推估**：過去 12 個月的每次配息，往後推一年（同月）。同一檔在推估除息日
//!   前後 [`MATCH_WINDOW_DAYS`] 天內已有公告的配息時不重複計算。金額依序取：
//!   1. 下一期金額已公告、只差日期 → 公告金額（[`Basis::AmountAnnounced`]）。
//!   2. 季配、月配 → 最近一期已公告的金額（[`Basis::LatestAmount`]）：每期金額跟著近期
//!      盈餘與配息政策走，回測 2023～2025 年誤差中位數季配 13.3% → 6.0%、月配 23.8% → 1.5%。
//!   3. 年配、半年配 → 去年同期金額（[`Basis::Estimated`]）。
//! - **月配依間隔補上**：月配股票從最近一期除息日依最近兩期的間隔往後排到預估期末
//!   （[`Basis::Cadence`]），補上去年同期沒有配息的月份（例如新上市的月配 ETF）；與已公告或
//!   已推估的配息相差 [`CADENCE_WINDOW_DAYS`] 天內的不重複。
//! - 同一除息日有多列（季配股的全年彙總列、月配 ETF 的季別與月別）只算一次，優先取各期列。
//! - **已除息、未發放**：已有已領股利明細（[`Entitlement`]）時，股數與金額取明細（除息日
//!   當時的持股），不用目前持股；除息後才買進或已賣出的部位因此不會算錯。
//!
//! 只算現金股利，不含股票股利，也不扣二代健保補充保費與所得稅；尚未除息的配息假設目前
//! 持股會一路持有到除息日。

use std::collections::HashMap;

use chrono::{Datelike, Days, Months, NaiveDate};
use rust_decimal::{Decimal, prelude::ToPrimitive};

use super::payout::PayoutAllocation;

/// 沒有發放日時，以除息日後幾天推估發放日（台股多在除息後 3～5 週發放）。
pub const PAYABLE_LAG_DAYS: u64 = 30;
/// 推估的除息日與已公告配息相差幾天內視為同一期。
pub const MATCH_WINDOW_DAYS: i64 = 60;
/// 月配依間隔排出的除息日與既有配息相差幾天內視為同一期。
pub const CADENCE_WINDOW_DAYS: i64 = 20;

/// 目前持有的股票。
#[derive(Debug, Clone, PartialEq)]
pub struct Holding {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 持有股數。
    pub shares: i64,
    /// 最新收盤價；沒有報價時為 `None`。
    pub closing_price: Option<Decimal>,
}

/// 股利表中的一列現金股利。
#[derive(Debug, Clone, PartialEq)]
pub struct DividendEvent {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股利所屬年度。
    pub year_of_dividend: i32,
    /// 期別（`Q1`～`Q4`、`M1`～`M12`、`H1`／`H2`；空字串為年配或全年彙總）。
    pub quarter: String,
    /// 每股現金股利（元）。
    pub cash_per_share: Decimal,
    /// 除息日；尚未公布為 `None`。
    pub ex_date: Option<NaiveDate>,
    /// 發放日；尚未公布為 `None`。
    pub payable_date: Option<NaiveDate>,
}

/// 已除息、已有已領股利明細的一筆現金股利（各會員合計）。
#[derive(Debug, Clone, PartialEq)]
pub struct Entitlement {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 除息日。
    pub ex_date: NaiveDate,
    /// 發放日。
    pub pay_date: NaiveDate,
    /// 每股現金股利。
    pub cash_per_share: Decimal,
    /// 除息日參與分配的股數。
    pub shares: i64,
    /// 已領股利明細的現金金額。
    pub amount: Decimal,
}

impl Entitlement {
    fn to_item(&self) -> ForecastItem {
        ForecastItem {
            stock_symbol: self.stock_symbol.clone(),
            name: self.name.clone(),
            shares: self.shares,
            ex_date: self.ex_date,
            pay_date: self.pay_date,
            cash_per_share: self.cash_per_share,
            amount: self.amount,
            basis: Basis::Announced,
            entitled: true,
        }
    }
}

/// 由股利入帳分配取出已除息、有已領明細的現金股利，依（代號, 除息日）合併各會員。
///
/// 以目前持股推算（`estimated`）、沒有現金、缺除息日或發放日的列不取；同一除息日有多列
/// （例如月配 ETF 的季別與月別）時取金額大者，與 [`forecast`] 對股利列的去重一致。
pub fn entitlements(allocations: &[PayoutAllocation]) -> Vec<Entitlement> {
    let mut by_row: HashMap<(&str, NaiveDate, &str), Entitlement> = HashMap::new();
    for allocation in allocations {
        if allocation.estimated || allocation.cash_amount <= Decimal::ZERO {
            continue;
        }
        let (Some(ex_date), Some(pay_date)) = (allocation.cash_ex, allocation.cash_payable) else {
            continue;
        };
        let key = (
            allocation.stock_symbol.as_str(),
            ex_date,
            allocation.quarter.as_str(),
        );
        let entry = by_row.entry(key).or_insert_with(|| Entitlement {
            stock_symbol: allocation.stock_symbol.clone(),
            name: allocation.name.clone(),
            ex_date,
            pay_date,
            cash_per_share: allocation.cash_dividend,
            shares: 0,
            amount: Decimal::ZERO,
        });
        entry.shares += allocation.shares;
        entry.amount += allocation.cash_amount;
    }
    let mut rows: Vec<Entitlement> = by_row.into_values().collect();
    rows.sort_by(|a, b| {
        (&a.stock_symbol, a.ex_date)
            .cmp(&(&b.stock_symbol, b.ex_date))
            .then_with(|| b.amount.cmp(&a.amount))
    });
    rows.dedup_by(|later, kept| {
        later.stock_symbol == kept.stock_symbol && later.ex_date == kept.ex_date
    });
    rows
}

/// 預估依據。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// 除息日與金額都已公告。
    Announced,
    /// 金額已公告、日期依去年同期推估。
    AmountAnnounced,
    /// 依去年同期推估（年配、半年配）。
    Estimated,
    /// 金額依最近一期已公告的配息、日期依去年同期推估（季配、月配）。
    LatestAmount,
    /// 月配依最近兩期的間隔排出日期、金額依最近一期推估。
    Cadence,
}

impl Basis {
    /// API 與快照表使用的 snake_case 名稱。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Announced => "announced",
            Self::AmountAnnounced => "amount_announced",
            Self::Estimated => "estimated",
            Self::LatestAmount => "latest_amount",
            Self::Cadence => "cadence",
        }
    }

    /// 是否為推估（日期與金額至少有一項未公告）。
    #[must_use]
    pub fn is_estimated(self) -> bool {
        matches!(self, Self::Estimated | Self::LatestAmount | Self::Cadence)
    }
}

/// 單筆預估配息。
#[derive(Debug, Clone, PartialEq)]
pub struct ForecastItem {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 持有股數。
    pub shares: i64,
    /// 除息日（推估者為去年同期加一年）。
    pub ex_date: NaiveDate,
    /// 發放日（已公告者為公告日，否則為推估）。
    pub pay_date: NaiveDate,
    /// 每股現金股利。
    pub cash_per_share: Decimal,
    /// 預估入帳金額＝股數 × 每股現金股利。
    pub amount: Decimal,
    /// 預估依據。
    pub basis: Basis,
    /// 已除息，股數與金額取已領股利明細（除息日持股）。
    pub entitled: bool,
}

/// 單月合計。
#[derive(Debug, Clone, PartialEq)]
pub struct MonthTotal {
    /// 月份，格式 `YYYY-MM`。
    pub month: String,
    /// 已公告（含金額已公告）的金額。
    pub announced: f64,
    /// 推估的金額（[`Basis::is_estimated`]）。
    pub estimated: f64,
}

/// 股利現金流預估結果。
#[derive(Debug, Clone, PartialEq)]
pub struct Forecast {
    /// 預估起日（不含）。
    pub from: NaiveDate,
    /// 預估迄日（含）。
    pub to: NaiveDate,
    /// 預估期間合計。
    pub total: f64,
    /// 其中已公告（含金額已公告）的金額。
    pub announced_total: f64,
    /// 其中推估的金額（[`Basis::is_estimated`]）。
    pub estimated_total: f64,
    /// 預估股利 ÷ 目前市值（%）；沒有市值時為 `None`。
    pub forward_yield_percent: Option<f64>,
    /// 各月合計（含 0 的月份），依月份由舊到新。
    pub months: Vec<MonthTotal>,
    /// 各筆預估，依發放日排序。
    pub items: Vec<ForecastItem>,
}

/// 預估 `today` 之後 `months` 個月內的股利現金流。
///
/// `entitlements` 為已除息、有已領明細的配息（[`entitlements`]）；同代號同除息日的已公告
/// 配息改用明細，不再以目前持股計算。
pub fn forecast(
    holdings: &[Holding],
    events: &[DividendEvent],
    entitlements: &[Entitlement],
    today: NaiveDate,
    months: u32,
) -> Forecast {
    let end = today
        .checked_add_months(Months::new(months))
        .unwrap_or(today);
    let lookback = today.checked_sub_months(Months::new(12)).unwrap_or(today);
    let in_window = |date: NaiveDate| date > today && date <= end;

    let mut items = Vec::new();
    for holding in holdings.iter().filter(|holding| holding.shares > 0) {
        let own: Vec<&DividendEvent> = events
            .iter()
            .filter(|event| event.stock_symbol == holding.stock_symbol)
            .collect();
        let dated = dedupe_dated(&own);
        let periodic: Vec<&DividendEvent> = dated
            .iter()
            .copied()
            .filter(|event| is_periodic(&event.quarter))
            .collect();
        let latest = periodic.last().copied();
        let item = |ex_date, pay_date, cash_per_share: Decimal, basis| ForecastItem {
            stock_symbol: holding.stock_symbol.clone(),
            name: holding.name.clone(),
            shares: holding.shares,
            ex_date,
            pay_date,
            cash_per_share,
            amount: cash_per_share * Decimal::from(holding.shares),
            basis,
            entitled: false,
        };

        for event in &dated {
            let (ex_date, pay_date) = (event.ex_date.unwrap_or(today), pay_date(event));
            let recorded = entitlements.iter().any(|entitlement| {
                entitlement.stock_symbol == holding.stock_symbol && entitlement.ex_date == ex_date
            });
            if in_window(pay_date) && !recorded {
                items.push(item(
                    ex_date,
                    pay_date,
                    event.cash_per_share,
                    Basis::Announced,
                ));
            }
        }

        for past in dated
            .iter()
            .filter(|event| event.ex_date.is_some_and(|ex| ex > lookback && ex <= today))
        {
            let (Some(ex_date), Some(pay_date)) =
                (past.ex_date.and_then(next_year), next_year(pay_date(past)))
            else {
                continue;
            };
            if !in_window(pay_date) {
                continue;
            }
            let already_announced = dated.iter().any(|event| {
                event
                    .ex_date
                    .is_some_and(|ex| (ex - ex_date).num_days().abs() <= MATCH_WINDOW_DAYS)
            });
            if already_announced {
                continue;
            }
            let announced_amount = own.iter().find(|event| {
                event.ex_date.is_none()
                    && event.quarter == past.quarter
                    && event.year_of_dividend == past.year_of_dividend + 1
            });
            let newer = latest
                .filter(|latest| is_periodic(&past.quarter) && latest.ex_date != past.ex_date);
            let (cash, basis) = match (announced_amount, newer) {
                (Some(event), _) => (event.cash_per_share, Basis::AmountAnnounced),
                (None, Some(latest)) => (latest.cash_per_share, Basis::LatestAmount),
                (None, None) => (past.cash_per_share, Basis::Estimated),
            };
            items.push(item(ex_date, pay_date, cash, basis));
        }

        let pays_monthly = own.iter().any(|event| {
            event.quarter.starts_with('M') && event.ex_date.is_some_and(|ex| ex > lookback)
        });
        if !pays_monthly {
            continue;
        }
        let Some(latest) = latest else {
            continue;
        };
        for (ex_date, pay_date) in cadence_schedule(&periodic, today, end) {
            let near = |other: NaiveDate| (other - ex_date).num_days().abs() <= CADENCE_WINDOW_DAYS;
            let covered = dated.iter().any(|event| event.ex_date.is_some_and(near))
                || items
                    .iter()
                    .any(|item| item.stock_symbol == holding.stock_symbol && near(item.ex_date));
            if !covered {
                items.push(item(
                    ex_date,
                    pay_date,
                    latest.cash_per_share,
                    Basis::Cadence,
                ));
            }
        }
    }
    let recorded = entitlements
        .iter()
        .filter(|entitlement| in_window(entitlement.pay_date));
    items.extend(recorded.map(Entitlement::to_item));
    items.sort_by(|a, b| {
        a.pay_date
            .cmp(&b.pay_date)
            .then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
    });

    let mut by_month: HashMap<String, (Decimal, Decimal)> = HashMap::new();
    for item in &items {
        let entry = by_month.entry(month_key(item.pay_date)).or_default();
        if item.basis.is_estimated() {
            entry.1 += item.amount;
        } else {
            entry.0 += item.amount;
        }
    }
    let month_list = month_range(today, end)
        .into_iter()
        .map(|month| {
            let (announced, estimated) = by_month.get(&month).copied().unwrap_or_default();
            MonthTotal {
                month,
                announced: money(announced),
                estimated: money(estimated),
            }
        })
        .collect();

    let announced_total: Decimal = items
        .iter()
        .filter(|item| !item.basis.is_estimated())
        .map(|item| item.amount)
        .sum();
    let estimated_total: Decimal = items
        .iter()
        .filter(|item| item.basis.is_estimated())
        .map(|item| item.amount)
        .sum();
    let total = announced_total + estimated_total;
    let market_value: Decimal = holdings
        .iter()
        .filter_map(|holding| {
            holding
                .closing_price
                .filter(|price| *price > Decimal::ZERO)
                .map(|price| price * Decimal::from(holding.shares))
        })
        .sum();

    Forecast {
        from: today,
        to: end,
        total: money(total),
        announced_total: money(announced_total),
        estimated_total: money(estimated_total),
        forward_yield_percent: (!market_value.is_zero())
            .then(|| money(total / market_value * Decimal::ONE_HUNDRED)),
        months: month_list,
        items,
    }
}

/// 有除息日的列，同一除息日只留一列：優先各期列（期別非空），再取金額大者。
fn dedupe_dated<'a>(events: &[&'a DividendEvent]) -> Vec<&'a DividendEvent> {
    let mut best: HashMap<NaiveDate, &DividendEvent> = HashMap::new();
    for event in events {
        let Some(ex_date) = event.ex_date else {
            continue;
        };
        let rank =
            |candidate: &DividendEvent| (!candidate.quarter.is_empty(), candidate.cash_per_share);
        best.entry(ex_date)
            .and_modify(|current| {
                if rank(event) > rank(current) {
                    *current = event;
                }
            })
            .or_insert(event);
    }
    let mut dated: Vec<&DividendEvent> = best.into_values().collect();
    dated.sort_by_key(|event| event.ex_date);
    dated
}

/// 季配（`Q`）與月配（`M`）的期別；年配（空白）與半年配（`H`）不算。
fn is_periodic(quarter: &str) -> bool {
    quarter.starts_with('Q') || quarter.starts_with('M')
}

/// 月配依間隔往後排的（除息日, 發放日），只取發放日落在 `today` 之後、`end` 以前者。
///
/// 間隔取最近兩期除息日相差的月數（至少 1 個月；只有一期時當作 1 個月），發放日沿用
/// 最近一期「除息到發放」的天數。最近一期已超過兩個間隔沒有配息時視為停配，不排。
fn cadence_schedule(
    periodic: &[&DividendEvent],
    today: NaiveDate,
    end: NaiveDate,
) -> Vec<(NaiveDate, NaiveDate)> {
    let Some((latest, earlier)) = periodic.split_last() else {
        return Vec::new();
    };
    let Some(latest_ex) = latest.ex_date else {
        return Vec::new();
    };
    let step = earlier
        .last()
        .and_then(|previous| previous.ex_date)
        .map_or(1, |previous_ex| {
            months_between(previous_ex, latest_ex).max(1)
        });
    let stale = latest_ex
        .checked_add_months(Months::new(step * 2))
        .is_none_or(|limit| limit < today);
    if stale {
        return Vec::new();
    }
    let pay_gap = pay_date(latest) - latest_ex;

    let mut schedule = Vec::new();
    for count in 1.. {
        let Some(ex_date) = latest_ex.checked_add_months(Months::new(step * count)) else {
            break;
        };
        let pay_date = ex_date + pay_gap;
        if pay_date > end {
            break;
        }
        if pay_date > today {
            schedule.push((ex_date, pay_date));
        }
    }
    schedule
}

/// `from` 到 `to` 相差的月數（只看年月）。
fn months_between(from: NaiveDate, to: NaiveDate) -> u32 {
    let months = (to.year() - from.year()) * 12 + to.month() as i32 - from.month() as i32;
    u32::try_from(months).unwrap_or_default()
}

/// 發放日：已公告用公告日，否則除息日後 [`PAYABLE_LAG_DAYS`] 天。
fn pay_date(event: &DividendEvent) -> NaiveDate {
    event.payable_date.unwrap_or_else(|| {
        let ex_date = event.ex_date.unwrap_or_default();
        ex_date + Days::new(PAYABLE_LAG_DAYS)
    })
}

fn next_year(date: NaiveDate) -> Option<NaiveDate> {
    date.checked_add_months(Months::new(12))
}

fn month_key(date: NaiveDate) -> String {
    format!("{:04}-{:02}", date.year(), date.month())
}

/// `today` 所在月份到 `end` 所在月份（含）的每個月。
fn month_range(today: NaiveDate, end: NaiveDate) -> Vec<String> {
    let mut months = Vec::new();
    let mut cursor = today.with_day(1).unwrap_or(today);
    while cursor <= end {
        months.push(month_key(cursor));
        match cursor.checked_add_months(Months::new(1)) {
            Some(next) => cursor = next,
            None => break,
        }
    }
    months
}

fn money(value: Decimal) -> f64 {
    value.round_dp(2).to_f64().unwrap_or_default()
}

#[cfg(test)]
mod tests;
