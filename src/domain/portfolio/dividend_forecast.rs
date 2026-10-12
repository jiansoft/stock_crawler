//! # 持股股利現金流預估
//!
//! 以目前持股股數，預估未來幾個月每月會收到多少現金股利：
//!
//! - **已公告**：除息日已公布、發放日落在預估期間內的配息；沒有發放日時以除息日後
//!   [`PAYABLE_LAG_DAYS`] 天推估。
//! - **推估**：過去 12 個月的每次配息，往後推一年（同月、同金額）。同一檔在推估除息日
//!   前後 [`MATCH_WINDOW_DAYS`] 天內已有公告的配息時不重複計算；下一期金額已公告、只差
//!   日期時改用公告金額（[`Basis::AmountAnnounced`]）。
//! - 同一除息日有多列（季配股的全年彙總列、月配 ETF 的季別與月別）只算一次，優先取各期列。
//!
//! 只算現金股利，不含股票股利，也不扣二代健保補充保費與所得稅；假設目前持股會一路
//! 持有到各次除息日。

use std::collections::HashMap;

use chrono::{Datelike, Days, Months, NaiveDate};
use rust_decimal::{Decimal, prelude::ToPrimitive};

/// 沒有發放日時，以除息日後幾天推估發放日（台股多在除息後 3～5 週發放）。
pub const PAYABLE_LAG_DAYS: u64 = 30;
/// 推估的除息日與已公告配息相差幾天內視為同一期。
pub const MATCH_WINDOW_DAYS: i64 = 60;

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

/// 預估依據。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// 除息日與金額都已公告。
    Announced,
    /// 金額已公告、日期依去年同期推估。
    AmountAnnounced,
    /// 依去年同期推估。
    Estimated,
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
}

/// 單月合計。
#[derive(Debug, Clone, PartialEq)]
pub struct MonthTotal {
    /// 月份，格式 `YYYY-MM`。
    pub month: String,
    /// 已公告（含金額已公告）的金額。
    pub announced: f64,
    /// 依去年同期推估的金額。
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
    /// 其中依去年同期推估的金額。
    pub estimated_total: f64,
    /// 預估股利 ÷ 目前市值（%）；沒有市值時為 `None`。
    pub forward_yield_percent: Option<f64>,
    /// 各月合計（含 0 的月份），依月份由舊到新。
    pub months: Vec<MonthTotal>,
    /// 各筆預估，依發放日排序。
    pub items: Vec<ForecastItem>,
}

/// 預估 `today` 之後 `months` 個月內的股利現金流。
pub fn forecast(
    holdings: &[Holding],
    events: &[DividendEvent],
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
        let item = |ex_date, pay_date, cash_per_share: Decimal, basis| ForecastItem {
            stock_symbol: holding.stock_symbol.clone(),
            name: holding.name.clone(),
            shares: holding.shares,
            ex_date,
            pay_date,
            cash_per_share,
            amount: cash_per_share * Decimal::from(holding.shares),
            basis,
        };

        for event in &dated {
            let (ex_date, pay_date) = (event.ex_date.unwrap_or(today), pay_date(event));
            if in_window(pay_date) {
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
            let (cash, basis) = match announced_amount {
                Some(event) => (event.cash_per_share, Basis::AmountAnnounced),
                None => (past.cash_per_share, Basis::Estimated),
            };
            items.push(item(ex_date, pay_date, cash, basis));
        }
    }
    items.sort_by(|a, b| {
        a.pay_date
            .cmp(&b.pay_date)
            .then_with(|| a.stock_symbol.cmp(&b.stock_symbol))
    });

    let mut by_month: HashMap<String, (Decimal, Decimal)> = HashMap::new();
    for item in &items {
        let entry = by_month.entry(month_key(item.pay_date)).or_default();
        match item.basis {
            Basis::Estimated => entry.1 += item.amount,
            Basis::Announced | Basis::AmountAnnounced => entry.0 += item.amount,
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
        .filter(|item| item.basis != Basis::Estimated)
        .map(|item| item.amount)
        .sum();
    let estimated_total: Decimal = items
        .iter()
        .filter(|item| item.basis == Basis::Estimated)
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
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("有效日期")
    }

    fn holding(symbol: &str, shares: i64, price: Option<Decimal>) -> Holding {
        Holding {
            stock_symbol: symbol.to_owned(),
            name: format!("{symbol} 名稱"),
            shares,
            closing_price: price,
        }
    }

    fn event(
        symbol: &str,
        year_of_dividend: i32,
        quarter: &str,
        cash: Decimal,
        ex_date: Option<NaiveDate>,
        payable_date: Option<NaiveDate>,
    ) -> DividendEvent {
        DividendEvent {
            stock_symbol: symbol.to_owned(),
            year_of_dividend,
            quarter: quarter.to_owned(),
            cash_per_share: cash,
            ex_date,
            payable_date,
        }
    }

    /// 已公告的配息直接列入；去年同期推一年；已公告的同期不重複；全年彙總列不計。
    #[test]
    fn announced_and_estimated_without_double_counting() {
        let today = date(2026, 10, 12);
        let events = [
            // 已公告：10-22 除息、11-11 發放。
            event(
                "0056",
                2026,
                "Q3",
                dec!(1.72),
                Some(date(2026, 10, 22)),
                Some(date(2026, 11, 11)),
            ),
            // 去年同期（2025-10-23）推到 2026-10-23，與已公告的 10-22 同期 → 不重複。
            event(
                "0056",
                2025,
                "Q3",
                dec!(0.866),
                Some(date(2025, 10, 23)),
                Some(date(2025, 11, 14)),
            ),
            // 去年 Q4（2026-01-22）推到 2027-01-22、發放 2027-02-11。
            event(
                "0056",
                2025,
                "Q4",
                dec!(0.866),
                Some(date(2026, 1, 22)),
                Some(date(2026, 2, 11)),
            ),
            // 全年彙總列沒有日期、期別空白 → 不計。
            event("0056", 2025, "", dec!(4.936), None, None),
            // 已發放過的不列入已公告。
            event(
                "0056",
                2026,
                "Q2",
                dec!(1.35),
                Some(date(2026, 7, 21)),
                Some(date(2026, 8, 10)),
            ),
        ];
        let forecast = forecast(&[holding("0056", 1000, Some(dec!(40)))], &events, today, 12);

        let summary: Vec<(NaiveDate, Decimal, Basis)> = forecast
            .items
            .iter()
            .map(|item| (item.pay_date, item.amount, item.basis))
            .collect();
        assert_eq!(
            summary,
            vec![
                (date(2026, 11, 11), dec!(1720), Basis::Announced),
                (date(2027, 2, 11), dec!(866), Basis::Estimated),
                // 去年 Q2（2026-07-21）推到 2027-07-21、發放 2027-08-10。
                (date(2027, 8, 10), dec!(1350), Basis::Estimated),
            ]
        );
        assert_eq!(forecast.total, 3936.0);
        assert_eq!(
            (forecast.announced_total, forecast.estimated_total),
            (1720.0, 2216.0)
        );
        // 市值 40,000 → 9.84%。
        assert_eq!(forecast.forward_yield_percent, Some(9.84));
        assert_eq!(forecast.months.len(), 13);
        assert_eq!(forecast.months[0].month, "2026-10");
        let november = forecast
            .months
            .iter()
            .find(|m| m.month == "2026-11")
            .expect("11 月");
        assert_eq!((november.announced, november.estimated), (1720.0, 0.0));
    }

    /// 下一期金額已公告、除息日未定：用公告金額、日期依去年；沒有發放日以除息後 30 天推估。
    #[test]
    fn amount_announced_and_missing_payable_date() {
        let today = date(2026, 3, 1);
        let events = [
            event("2887", 2024, "", dec!(0.5), Some(date(2025, 7, 24)), None),
            event("2887", 2025, "", dec!(0.6), None, None),
        ];
        let forecast = forecast(&[holding("2887", 2000, None)], &events, today, 12);
        assert_eq!(forecast.items.len(), 1);
        let item = &forecast.items[0];
        assert_eq!(item.basis, Basis::AmountAnnounced);
        assert_eq!(
            (item.ex_date, item.pay_date),
            (date(2026, 7, 24), date(2026, 8, 23))
        );
        assert_eq!(item.amount, dec!(1200));
        assert_eq!(forecast.announced_total, 1200.0);
        assert_eq!(forecast.forward_yield_percent, None);
    }

    /// 同一除息日的季別與月別兩列只算一次；股數 0、沒有配息紀錄的持股不產生項目。
    #[test]
    fn duplicate_rows_and_empty_holdings() {
        let today = date(2026, 10, 1);
        let ex = Some(date(2026, 10, 16));
        let pay = Some(date(2026, 11, 13));
        let events = [
            event("00878", 2026, "Q3", dec!(0.4), ex, pay),
            event("00878", 2026, "M9", dec!(0.4), ex, pay),
        ];
        let holdings = [
            holding("00878", 1000, Some(dec!(20))),
            holding("9999", 500, Some(dec!(10))),
            holding("1101", 0, Some(dec!(30))),
        ];
        let forecast = forecast(&holdings, &events, today, 3);
        assert_eq!(forecast.items.len(), 1);
        assert_eq!(forecast.total, 400.0);
        assert_eq!(forecast.months.len(), 4);

        let empty = super::forecast(&[], &[], today, 1);
        assert_eq!(
            (empty.total, empty.items.len(), empty.months.len()),
            (0.0, 0, 2)
        );
        assert_eq!(empty.forward_yield_percent, None);
    }
}
