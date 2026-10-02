//! # 以交易所除權除息計算結果核對股利資料
//!
//! Yahoo 是股利明細的主要來源，但它會把延後除息前的舊日期留在頁面上（1109 2022 年同時列
//! 07-14 與 08-04），對 ETF 的期別也常標錯。證交所 `TWT49U` 與櫃買 `exDailyQ` 是交易所依實際
//! 除權息日計算參考價的結果，日期與金額最可靠，這個流程用它們回頭核對 `dividend`。
//!
//! ## 比對規則（每檔股票分開比對，年度合計列不參與）
//!
//! 1. **同日**：資料列的除息日（現金）或除權日（股票）與交易所相同 → 已收錄。只有一列
//!    同日時，在金額差距不大（20% 以內）下以交易所金額修正（1.5 → 1.5035）；同一天有多列
//!    （2505 國揚 2020-08-18 同時除 2019 年配 0.15 與 2020Q2 1.5，交易所息值是合計 1.65）
//!    時不動金額，否則合計會被重複計入。
//! 2. **日期延後**（只限個股）：資料列的日期早於核對截止日、不是任何一次交易所除權息日、
//!    與交易所日期相差 60 天內、金額相近（20% 以內或資料列為 0）且只有這一列符合 →
//!    改成交易所日期與金額。截止日之後的日期是尚未發生的事件，不是舊日期；ETF 配息間隔
//!    常在 60 天內，搬日期容易配錯月份，一律不搬。
//! 3. **日期未公布**：資料列日期仍是「尚未公布」或 `-`、發放年度相符、金額差 1% 以內且只有
//!    這一列符合 → 補上日期。
//! 4. **資料庫沒有**：交易所沒有股利所屬期間，無從決定該寫進哪一列；寫入模式下到 Yahoo
//!    找同一天除權息的那筆明細，取它的所屬年度與期別補進來（2236 2024 年的第二次 H2、
//!    006208 的 11 月配息）。目標鍵上已有一列且那列已對上另一次交易所事件時不覆蓋。
//!    Yahoo 也找不到的只回報。
//! 5. **配股金額缺漏**：證交所「權息」拆不出股票股利，對上的資料列若配股為 0（2327 國巨
//!    2024-08-15 只記了現金），寫入模式下用 Yahoo 同一個除權日的配股金額補上。
//!
//! 一筆資料列只會對應到一次交易所事件；同日比對先全部處理完，才進行 2、3，
//! 避免一年配兩次（006208 的 7 月與 11 月）時把第二次事件誤配到第一次的資料列上。
//!
//! 證交所的「權」「權息」拆不出現金與股票股利，只用日期比對、不改金額；
//! 單純「權」也可能是現金增資，因此對不上時不列入缺漏。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;

use crate::{
    app::backfill::acl::YahooDividendAclMapper,
    app::calculation::dividend_record,
    domain::dividend::{entity::Dividend, repository::DividendRepository},
    infra::{
        crawler::{share::ExDividendAnnouncement, tpex, twse, yahoo},
        database::repository::dividend::PgDividendRepository,
    },
};

/// 日期延後時，資料列日期與交易所日期最多相差的天數。
const SHIFT_WINDOW_DAYS: i64 = 60;
/// 判定「金額相近」的相對差距上限。
const AMOUNT_TOLERANCE: Decimal = Decimal::from_parts(2, 0, 0, false, 1);
/// 日期未公布時，金額必須幾乎相同（相對差距 1% 以內）才敢對應。
const UNANNOUNCED_TOLERANCE: Decimal = Decimal::from_parts(1, 0, 0, false, 2);
/// 日期格式。
const DATE_FORMAT: &str = "%Y-%m-%d";

/// 核對結果摘要。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReconcileSummary {
    /// 交易所除權息事件數。
    pub official: usize,
    /// 資料庫已有同日事件的筆數。
    pub matched: usize,
    /// 改寫（日期或金額）的資料列數。
    pub updated: usize,
    /// 資料庫找不到對應事件的筆數（只回報）。
    pub missing: usize,
    /// 有多列可能對應、無法判斷的筆數（只回報）。
    pub ambiguous: usize,
    /// 從 Yahoo 補進來的事件數（只在寫入模式）。
    pub filled: usize,
    /// 從 Yahoo 補上配股金額的資料列數（只在寫入模式）。
    pub stock_filled: usize,
}

/// 一次核對要做的異動。
#[derive(Debug, Default)]
pub(crate) struct ReconcilePlan {
    /// 要改寫的資料列；key 為 serial。
    pub(crate) updates: BTreeMap<i64, Dividend>,
    /// 已有同日事件的交易所事件數。
    pub(crate) matched: usize,
    /// 找不到對應的交易所事件。
    pub(crate) missing: Vec<ExDividendAnnouncement>,
    /// 無法判斷對應哪一列的交易所事件。
    pub(crate) ambiguous: Vec<ExDividendAnnouncement>,
    /// 已對上交易所事件的資料列 serial，補缺漏時不可覆蓋。
    pub(crate) used: HashSet<i64>,
    /// 對上了、但配股金額缺而交易所又沒給的 (serial, 事件)，寫入模式下用 Yahoo 補。
    pub(crate) stock_unknown: Vec<(i64, ExDividendAnnouncement)>,
}

/// 抓取 `start`～`end` 的交易所除權息結果並核對資料庫；`apply` 為 false 時只回報不寫入。
///
/// # 錯誤
///
/// 任一交易所來源抓取失敗、或資料庫讀寫失敗時回傳錯誤；來源失敗時不寫入任何資料。
pub async fn execute(start: NaiveDate, end: NaiveDate, apply: bool) -> Result<ReconcileSummary> {
    let mut official = Vec::new();
    for (from, to) in calendar_year_chunks(start, end) {
        official.extend(twse::ex_right_result::visit(from, to).await?);
        official.extend(tpex::ex_right_result::visit(from, to).await?);
    }

    let repo = PgDividendRepository::new();
    // 跨年發放（12 月除息、隔年 1 月發放）的資料列在下一個發放年度，前後各多讀一年。
    let years: Vec<i32> = (start.year() - 1..=end.year() + 1).collect();
    let mut existing: HashMap<String, Vec<Dividend>> = HashMap::new();
    for row in repo
        .fetch_by_years(&years)
        .await
        .context("Failed to load dividends for reconciliation")?
    {
        existing
            .entry(row.security_code.clone())
            .or_default()
            .push(row);
    }

    let plan = build_plan(&official, &existing, end);
    let mut summary = ReconcileSummary {
        official: official.len(),
        matched: plan.matched,
        updated: plan.updates.len(),
        missing: plan.missing.len(),
        ambiguous: plan.ambiguous.len(),
        filled: 0,
        stock_filled: 0,
    };

    if !plan.missing.is_empty() {
        let sample: Vec<String> = plan
            .missing
            .iter()
            .take(20)
            .map(|event| format!("{} {}", event.stock_symbol, event.ex_date))
            .collect();
        tracing::warn!(
            count = plan.missing.len(),
            "交易所除權息事件在資料庫找不到對應：{}",
            sample.join(", ")
        );
    }

    if apply {
        apply_plan(&repo, &plan).await?;
        (summary.filled, summary.stock_filled) =
            fill_missing_from_yahoo(&repo, &plan, &existing).await?;
    }
    Ok(summary)
}

/// 用 Yahoo 補資料庫缺的部分：缺漏事件整筆補進來，配股金額缺漏的列補上配股。
/// 回傳 `(補入事件數, 補上配股的列數)`。
///
/// 每檔股票只抓一次 Yahoo、間隔 2 秒；Yahoo 抓取失敗（例如已下市的 404）只記錄並略過。
async fn fill_missing_from_yahoo(
    repo: &PgDividendRepository,
    plan: &ReconcilePlan,
    existing: &HashMap<String, Vec<Dividend>>,
) -> Result<(usize, usize)> {
    let mut by_symbol: BTreeMap<&str, Vec<&ExDividendAnnouncement>> = BTreeMap::new();
    for event in &plan.missing {
        by_symbol
            .entry(event.stock_symbol.as_str())
            .or_default()
            .push(event);
    }
    let mut stock_by_symbol: BTreeMap<&str, Vec<(i64, &ExDividendAnnouncement)>> = BTreeMap::new();
    for (serial, event) in &plan.stock_unknown {
        stock_by_symbol
            .entry(event.stock_symbol.as_str())
            .or_default()
            .push((*serial, event));
        by_symbol.entry(event.stock_symbol.as_str()).or_default();
    }
    let rows_by_serial: HashMap<i64, &Dividend> = existing
        .values()
        .flatten()
        .map(|row| (row.serial, row))
        .collect();

    let mut filled = 0usize;
    let mut stock_filled = 0usize;
    for (symbol, events) in by_symbol {
        let yahoo = match yahoo::dividend::visit(symbol).await {
            Ok(value) => value,
            Err(why) => {
                tracing::warn!("Yahoo 股利頁抓取失敗，略過補缺漏：{symbol} {why:#}");
                tokio::time::sleep(YAHOO_INTERVAL).await;
                continue;
            }
        };
        let details: Vec<Dividend> = yahoo
            .dividend
            .iter()
            .flat_map(|(_, details)| details)
            .map(|detail| {
                YahooDividendAclMapper::from_command(&YahooDividendAclMapper::from_dto(
                    symbol, detail,
                ))
            })
            .collect();
        let rows = existing.get(symbol).map(Vec::as_slice).unwrap_or_default();

        let mut total_years = BTreeSet::new();
        let mut filled_here = 0usize;
        for dividend in select_fills(&events, &details, rows, &plan.used) {
            repo.save(&dividend).await.with_context(|| {
                format!(
                    "Failed to save dividend filled from Yahoo {} {} {}",
                    dividend.security_code, dividend.year, dividend.quarter
                )
            })?;
            if !dividend.quarter.is_empty() {
                total_years.insert(dividend.year);
            }
            filled_here += 1;
        }
        for year in &total_years {
            repo.upsert_annual_total_dividend(symbol, *year).await?;
        }
        for (serial, event) in stock_by_symbol
            .get(symbol)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let base = plan
                .updates
                .get(serial)
                .or_else(|| rows_by_serial.get(serial).copied());
            let Some(fixed) = base.and_then(|row| stock_from_yahoo(row, event, &details)) else {
                continue;
            };
            repo.save(&fixed).await.with_context(|| {
                format!(
                    "Failed to save stock dividend from Yahoo {symbol} {}",
                    fixed.year
                )
            })?;
            if !fixed.quarter.is_empty() {
                repo.upsert_annual_total_dividend(symbol, fixed.year)
                    .await?;
            }
            stock_filled += 1;
            filled_here += 1;
        }
        filled += filled_here;
        if filled_here > 0 {
            dividend_record::backfill_received_dividend_records_for_stock(symbol)
                .await
                .with_context(|| format!("Failed to rebuild dividend records for {symbol}"))?;
        }
        tokio::time::sleep(YAHOO_INTERVAL).await;
    }
    Ok((filled, stock_filled))
}

/// 用 Yahoo 同一個除權日的配股金額補上資料列缺的股票股利；Yahoo 沒有就回傳 `None`。
pub(crate) fn stock_from_yahoo(
    row: &Dividend,
    event: &ExDividendAnnouncement,
    yahoo_details: &[Dividend],
) -> Option<Dividend> {
    let date = event.ex_date.format(DATE_FORMAT).to_string();
    let stock: Decimal = yahoo_details
        .iter()
        .filter(|detail| detail.ex_dividend_date_stock == date)
        .map(|detail| detail.stock_dividend)
        .sum();
    if stock <= Decimal::ZERO {
        return None;
    }
    let mut fixed = row.clone();
    fixed.stock_dividend = stock;
    fixed.sum = fixed.cash_dividend + stock;
    fixed.ex_dividend_date_stock = date;
    Some(fixed)
}

/// 對上的資料列配股為 0、交易所有除權卻沒給配股金額時，記下來留給 Yahoo 補。
fn note_stock_unknown(plan: &mut ReconcilePlan, row: &Dividend, event: &ExDividendAnnouncement) {
    if event.is_stock && event.stock_dividend_ratio.is_none() && row.stock_dividend.is_zero() {
        plan.stock_unknown.push((row.serial, event.clone()));
    }
}

/// 從 Yahoo 明細挑出與缺漏事件同一天除權息、可以安全寫入的列。
///
/// 目標鍵（年度層級列看發放年度，分期明細看主鍵）上已有一列、且那列已對上另一次交易所事件時
/// 不寫入，否則會把已確認的配息覆蓋掉。
pub(crate) fn select_fills(
    events: &[&ExDividendAnnouncement],
    yahoo_details: &[Dividend],
    rows: &[Dividend],
    used: &HashSet<i64>,
) -> Vec<Dividend> {
    let mut chosen: Vec<Dividend> = Vec::new();
    for event in events {
        let date = event.ex_date.format(DATE_FORMAT).to_string();
        let Some(detail) = yahoo_details.iter().find(|detail| {
            (event.is_cash && detail.ex_dividend_date_cash == date)
                || (event.is_stock && detail.ex_dividend_date_stock == date)
        }) else {
            continue;
        };
        let occupied = rows.iter().any(|row| {
            used.contains(&row.serial)
                && row.year == detail.year
                && row.quarter == detail.quarter
                && (detail.quarter.is_empty() || row.year_of_dividend == detail.year_of_dividend)
        });
        let duplicate = chosen.iter().any(|picked| {
            picked.year == detail.year
                && picked.quarter == detail.quarter
                && picked.year_of_dividend == detail.year_of_dividend
        });
        if !occupied && !duplicate {
            chosen.push(detail.clone());
        }
    }
    chosen
}

/// 逐檔抓 Yahoo 的間隔，降低被 WAF 封鎖的機率。
const YAHOO_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// 寫入核對結果：逐列 upsert，重算受影響的年度合計，再重算持股已領股利。
async fn apply_plan(repo: &PgDividendRepository, plan: &ReconcilePlan) -> Result<()> {
    let mut total_years: BTreeSet<(String, i32)> = BTreeSet::new();
    let mut symbols: BTreeSet<String> = BTreeSet::new();
    for dividend in plan.updates.values() {
        repo.save(dividend).await.with_context(|| {
            format!(
                "Failed to save reconciled dividend {} {} {}",
                dividend.security_code, dividend.year, dividend.quarter
            )
        })?;
        if !dividend.quarter.is_empty() {
            total_years.insert((dividend.security_code.clone(), dividend.year));
        }
        symbols.insert(dividend.security_code.clone());
    }
    for (symbol, year) in &total_years {
        repo.upsert_annual_total_dividend(symbol, *year).await?;
    }
    for symbol in &symbols {
        dividend_record::backfill_received_dividend_records_for_stock(symbol)
            .await
            .with_context(|| format!("Failed to rebuild dividend records for {symbol}"))?;
    }
    Ok(())
}

/// 依「同日 → 日期延後 → 日期未公布」的順序，把交易所事件對應到資料列並產生異動。
///
/// `as_of` 是交易所資料的截止日：資料列日期晚於它的是尚未發生的事件，不當成舊日期。
pub(crate) fn build_plan(
    official: &[ExDividendAnnouncement],
    existing: &HashMap<String, Vec<Dividend>>,
    as_of: NaiveDate,
) -> ReconcilePlan {
    let mut plan = ReconcilePlan::default();
    let mut used: HashSet<i64> = HashSet::new();
    let mut pending: Vec<&ExDividendAnnouncement> = Vec::new();

    // 每檔股票的交易所除權息日，用來判斷資料列的日期是不是「舊日期」。
    let mut official_dates: HashMap<&str, HashSet<String>> = HashMap::new();
    for event in official {
        official_dates
            .entry(event.stock_symbol.as_str())
            .or_default()
            .insert(event.ex_date.format(DATE_FORMAT).to_string());
    }

    for event in official {
        let rows = event_rows(existing, &event.stock_symbol);
        let date = event.ex_date.format(DATE_FORMAT).to_string();
        let same_day: Vec<&Dividend> = rows
            .iter()
            .filter(|row| {
                !used.contains(&row.serial)
                    && ((event.is_cash && row.ex_dividend_date_cash == date)
                        || (event.is_stock && row.ex_dividend_date_stock == date))
            })
            .copied()
            .collect();
        match same_day.as_slice() {
            [] => pending.push(event),
            [row] => {
                used.insert(row.serial);
                plan.matched += 1;
                let mut candidate = (*row).clone();
                let changed = apply_official(&mut candidate, event);
                note_stock_unknown(&mut plan, &candidate, event);
                if changed {
                    plan.updates.insert(candidate.serial, candidate);
                }
            }
            // 同一天多次配息：交易所息值是合計，逐列的金額無從拆分，只確認已收錄。
            rows_on_day => {
                used.extend(rows_on_day.iter().map(|row| row.serial));
                plan.matched += 1;
            }
        }
    }

    for event in pending {
        let rows = event_rows(existing, &event.stock_symbol);
        let stale_dates = official_dates
            .get(event.stock_symbol.as_str())
            .cloned()
            .unwrap_or_default();

        let is_etf = event.stock_symbol.starts_with("00");
        let shifted: Vec<&Dividend> = rows
            .iter()
            .filter(|_| !is_etf)
            .filter(|row| !used.contains(&row.serial))
            .filter(|row| {
                event_dates(row, event).iter().any(|date| {
                    parse_date(date).is_some_and(|row_date| {
                        row_date <= as_of
                            && !stale_dates.contains(*date)
                            && (row_date - event.ex_date).num_days().abs() <= SHIFT_WINDOW_DAYS
                    })
                })
            })
            .filter(|row| amounts_close(row, event, AMOUNT_TOLERANCE, true))
            .copied()
            .collect();

        let candidates: Vec<&Dividend> = if shifted.is_empty() {
            rows.iter()
                .filter(|row| !used.contains(&row.serial))
                .filter(|row| {
                    event_dates(row, event)
                        .iter()
                        .all(|date| parse_date(date).is_none())
                })
                .filter(|row| {
                    row.year == event.ex_date.year() || row.year == event.ex_date.year() + 1
                })
                .filter(|row| amounts_close(row, event, UNANNOUNCED_TOLERANCE, false))
                .copied()
                .collect()
        } else {
            shifted
        };

        match candidates.as_slice() {
            [row] => {
                used.insert(row.serial);
                let mut candidate = (*row).clone();
                set_event_dates(&mut candidate, event);
                apply_official(&mut candidate, event);
                note_stock_unknown(&mut plan, &candidate, event);
                plan.updates.insert(candidate.serial, candidate);
            }
            [] => {
                // 證交所單純「權」可能是現金增資，對不上不算缺漏。
                let rights_only = !event.is_cash && event.stock_dividend_ratio.is_none();
                if !rights_only {
                    plan.missing.push(event.clone());
                }
            }
            _ => plan.ambiguous.push(event.clone()),
        }
    }

    plan.used = used;
    plan
}

/// 一檔股票可參與比對的資料列：排除年度合計列（混合配息年度的空期別）。
fn event_rows<'a>(existing: &'a HashMap<String, Vec<Dividend>>, symbol: &str) -> Vec<&'a Dividend> {
    let rows = existing.get(symbol).map(Vec::as_slice).unwrap_or_default();
    rows.iter()
        .filter(|row| {
            !(row.quarter.is_empty()
                && rows
                    .iter()
                    .any(|other| other.year == row.year && !other.quarter.is_empty()))
        })
        .collect()
}

/// 交易所事件類型對應到資料列的日期欄：除息看現金欄、除權看股票欄。
fn event_dates<'a>(row: &'a Dividend, event: &ExDividendAnnouncement) -> Vec<&'a String> {
    let mut dates = Vec::with_capacity(2);
    if event.is_cash {
        dates.push(&row.ex_dividend_date_cash);
    }
    if event.is_stock {
        dates.push(&row.ex_dividend_date_stock);
    }
    dates
}

/// 金額是否相近：只比交易所有提供的金額；`allow_zero` 時資料列為 0 也算相近。
fn amounts_close(
    row: &Dividend,
    event: &ExDividendAnnouncement,
    tolerance: Decimal,
    allow_zero: bool,
) -> bool {
    let close = |stored: Decimal, official: Decimal| {
        (allow_zero && stored.is_zero())
            || (official > Decimal::ZERO && ((stored - official).abs() / official) <= tolerance)
    };
    let cash_ok = event
        .cash_dividend
        .is_none_or(|official| close(row.cash_dividend, official));
    let stock_ok = event
        .stock_dividend()
        .is_none_or(|official| close(row.stock_dividend, official));
    // 交易所沒給任何金額（證交所「權」「權息」）時，未公布日期的列無從比對，只接受日期延後。
    let has_amount = event.cash_dividend.is_some() || event.stock_dividend_ratio.is_some();
    cash_ok && stock_ok && (has_amount || allow_zero)
}

/// 把交易所日期寫進資料列對應的日期欄。
fn set_event_dates(row: &mut Dividend, event: &ExDividendAnnouncement) {
    let date = event.ex_date.format(DATE_FORMAT).to_string();
    if event.is_cash {
        row.ex_dividend_date_cash = date.clone();
    }
    if event.is_stock {
        row.ex_dividend_date_stock = date;
    }
}

/// 以交易所金額修正資料列；只在差距 20% 以內（或資料列為 0）時修正，回傳是否有變動。
///
/// 差太多代表可能是另一次配息被對上，寧可不改。
fn apply_official(row: &mut Dividend, event: &ExDividendAnnouncement) -> bool {
    let mut changed = false;
    let mut update = |stored: &mut Decimal, official: Option<Decimal>| {
        let Some(official) = official.filter(|value| *value > Decimal::ZERO) else {
            return;
        };
        let close = stored.is_zero() || ((*stored - official).abs() / official) <= AMOUNT_TOLERANCE;
        if close && (*stored - official).abs() > Decimal::new(5, 5) {
            *stored = official;
            changed = true;
        }
    };
    update(&mut row.cash_dividend, event.cash_dividend);
    update(
        &mut row.stock_dividend,
        event.stock_dividend().map(|value| value.round_dp(4)),
    );
    if changed {
        row.sum = row.cash_dividend + row.stock_dividend;
    }
    // 同日比對時日期已經相同；兩種股利同一天除權息時補上另一欄的日期。
    let date = event.ex_date.format(DATE_FORMAT).to_string();
    if event.is_cash
        && row.cash_dividend > Decimal::ZERO
        && parse_date(&row.ex_dividend_date_cash).is_none()
    {
        row.ex_dividend_date_cash = date.clone();
        changed = true;
    }
    if event.is_stock
        && row.stock_dividend > Decimal::ZERO
        && parse_date(&row.ex_dividend_date_stock).is_none()
    {
        row.ex_dividend_date_stock = date;
        changed = true;
    }
    changed
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, DATE_FORMAT).ok()
}

/// 把日期區間切成逐個日曆年，避免單次請求跨太多年。
fn calendar_year_chunks(start: NaiveDate, end: NaiveDate) -> Vec<(NaiveDate, NaiveDate)> {
    (start.year()..=end.year())
        .filter_map(|year| {
            let from = NaiveDate::from_ymd_opt(year, 1, 1)?.max(start);
            let to = NaiveDate::from_ymd_opt(year, 12, 31)?.min(end);
            (from <= to).then_some((from, to))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::Local;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::core::declare::StockExchangeMarket;

    fn row(
        serial: i64,
        year: i32,
        quarter: &str,
        cash: Decimal,
        stock: Decimal,
        ex_cash: &str,
        ex_stock: &str,
    ) -> Dividend {
        Dividend {
            serial,
            year,
            year_of_dividend: year - 1,
            quarter: quarter.to_string(),
            security_code: "1109".to_string(),
            earnings_cash_dividend: Decimal::ZERO,
            capital_reserve_cash_dividend: Decimal::ZERO,
            cash_dividend: cash,
            earnings_stock_dividend: Decimal::ZERO,
            capital_reserve_stock_dividend: Decimal::ZERO,
            stock_dividend: stock,
            sum: cash + stock,
            payout_ratio_cash: Decimal::ZERO,
            payout_ratio_stock: Decimal::ZERO,
            payout_ratio: Decimal::ZERO,
            ex_dividend_date_cash: ex_cash.to_string(),
            ex_dividend_date_stock: ex_stock.to_string(),
            payable_date_cash: "-".to_string(),
            payable_date_stock: "-".to_string(),
            created_time: Local::now(),
            updated_time: Local::now(),
        }
    }

    fn cash_event(date: &str, cash: Decimal) -> ExDividendAnnouncement {
        ExDividendAnnouncement {
            stock_symbol: "1109".to_string(),
            name: "信大".to_string(),
            ex_date: NaiveDate::parse_from_str(date, DATE_FORMAT).unwrap(),
            is_cash: true,
            is_stock: false,
            cash_dividend: Some(cash),
            stock_dividend_ratio: None,
            market: StockExchangeMarket::Listed,
        }
    }

    fn existing(rows: Vec<Dividend>) -> HashMap<String, Vec<Dividend>> {
        HashMap::from([("1109".to_string(), rows)])
    }

    /// 測試用的交易所資料截止日。
    const AS_OF: NaiveDate = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();

    /// 同一天兩次配息時，交易所息值是合計：兩列都算已收錄，金額不動（2505 國揚 2020-08-18）。
    #[test]
    fn two_rows_on_the_same_day_are_matched_without_amount_changes() {
        let rows = existing(vec![
            row(1, 2020, "A", dec!(0.15), Decimal::ZERO, "2020-08-18", "-"),
            row(2, 2020, "Q2", dec!(1.5), Decimal::ZERO, "2020-08-18", "-"),
        ]);
        let plan = build_plan(&[cash_event("2020-08-18", dec!(1.65))], &rows, AS_OF);

        assert_eq!(plan.matched, 1);
        assert!(plan.updates.is_empty());
        assert!(plan.missing.is_empty());
    }

    /// 截止日之後的日期是尚未發生的事件，不可以被當成舊日期搬走。
    #[test]
    fn future_dates_are_not_treated_as_stale() {
        let rows = existing(vec![row(
            1,
            2026,
            "",
            dec!(0.11),
            Decimal::ZERO,
            "2026-10-19",
            "-",
        )]);
        let plan = build_plan(&[cash_event("2026-09-16", dec!(0.11))], &rows, AS_OF);

        assert!(plan.updates.is_empty());
        assert_eq!(plan.missing.len(), 1);
    }

    /// ETF 配息間隔短，不做日期搬移（只做同日與未公布日期的比對）。
    #[test]
    fn etf_dates_are_never_shifted() {
        let mut stale = row(1, 2025, "M05", dec!(0.11), Decimal::ZERO, "2025-06-01", "-");
        stale.security_code = "00730".to_string();
        let rows = HashMap::from([("00730".to_string(), vec![stale])]);
        let event = ExDividendAnnouncement {
            stock_symbol: "00730".to_string(),
            ..cash_event("2025-06-17", dec!(0.11))
        };
        let plan = build_plan(&[event], &rows, AS_OF);

        assert!(plan.updates.is_empty());
        assert_eq!(plan.missing.len(), 1);
    }

    /// 同日已收錄：只把四捨五入過的金額換成交易所金額。
    #[test]
    fn same_day_event_only_corrects_the_amount() {
        let rows = existing(vec![row(
            1,
            2022,
            "",
            dec!(1.5),
            Decimal::ZERO,
            "2022-08-04",
            "-",
        )]);
        let plan = build_plan(&[cash_event("2022-08-04", dec!(1.5035))], &rows, AS_OF);

        assert_eq!(plan.matched, 1);
        let updated = plan.updates.get(&1).expect("金額應修正");
        assert_eq!(updated.cash_dividend, dec!(1.5035));
        assert_eq!(updated.sum, dec!(1.5035));
        assert!(plan.missing.is_empty());
    }

    /// 金額一樣、日期也一樣時不產生異動。
    #[test]
    fn identical_event_is_left_untouched() {
        let rows = existing(vec![row(
            1,
            2022,
            "",
            dec!(1.5035),
            Decimal::ZERO,
            "2022-08-04",
            "-",
        )]);
        let plan = build_plan(&[cash_event("2022-08-04", dec!(1.5035))], &rows, AS_OF);
        assert_eq!(plan.matched, 1);
        assert!(plan.updates.is_empty());
    }

    /// 延後除息前的舊日期（07-14）改成交易所日期（08-04）。
    #[test]
    fn stale_date_is_moved_to_the_official_date() {
        let rows = existing(vec![row(
            1,
            2022,
            "",
            dec!(1.5),
            Decimal::ZERO,
            "2022-07-14",
            "-",
        )]);
        let plan = build_plan(&[cash_event("2022-08-04", dec!(1.5035))], &rows, AS_OF);

        let updated = plan.updates.get(&1).expect("日期應改寫");
        assert_eq!(updated.ex_dividend_date_cash, "2022-08-04");
        assert_eq!(updated.cash_dividend, dec!(1.5035));
    }

    /// 一年配兩次但資料庫只有第一次：第二次不可以被配到第一次的資料列上（006208 的 11 月）。
    #[test]
    fn a_row_already_matching_an_official_date_is_not_moved() {
        let rows = existing(vec![row(
            1,
            2025,
            "H1",
            dec!(0.989),
            Decimal::ZERO,
            "2025-07-16",
            "-",
        )]);
        let plan = build_plan(
            &[
                cash_event("2025-07-16", dec!(0.989)),
                cash_event("2025-11-18", dec!(3.448)),
            ],
            &rows,
            AS_OF,
        );

        assert_eq!(plan.matched, 1);
        assert!(plan.updates.is_empty());
        assert_eq!(plan.missing.len(), 1);
        assert_eq!(
            plan.missing[0].ex_date,
            NaiveDate::from_ymd_opt(2025, 11, 18).unwrap()
        );
    }

    /// 日期還沒公布、金額相同的列補上日期。
    #[test]
    fn unannounced_row_gets_the_official_date() {
        let rows = existing(vec![row(
            1,
            2023,
            "H2",
            dec!(0.5),
            Decimal::ZERO,
            "尚未公布",
            "尚未公布",
        )]);
        let plan = build_plan(&[cash_event("2023-07-11", dec!(0.5))], &rows, AS_OF);

        let updated = plan.updates.get(&1).expect("應補上日期");
        assert_eq!(updated.ex_dividend_date_cash, "2023-07-11");
    }

    /// 兩列都可能是同一次配息時不猜，列為無法判斷。
    #[test]
    fn two_candidate_rows_are_ambiguous() {
        let rows = existing(vec![
            row(1, 2023, "H1", dec!(0.5), Decimal::ZERO, "尚未公布", "-"),
            row(2, 2023, "H2", dec!(0.5), Decimal::ZERO, "尚未公布", "-"),
        ]);
        let plan = build_plan(&[cash_event("2023-07-11", dec!(0.5))], &rows, AS_OF);

        assert!(plan.updates.is_empty());
        assert_eq!(plan.ambiguous.len(), 1);
    }

    /// 金額差太多不算同一次配息；年度合計列不參與比對。
    #[test]
    fn far_amounts_and_annual_totals_are_not_matched() {
        let rows = existing(vec![
            row(1, 2023, "", dec!(2.2), Decimal::ZERO, "-", "-"),
            row(2, 2023, "H1", dec!(5.0), Decimal::ZERO, "2023-06-01", "-"),
            row(3, 2023, "H2", dec!(1.6), Decimal::ZERO, "2023-11-29", "-"),
        ]);
        let plan = build_plan(&[cash_event("2023-06-19", dec!(1.6))], &rows, AS_OF);

        assert!(plan.updates.is_empty());
        assert_eq!(plan.missing.len(), 1);
    }

    /// 證交所單純「權」沒有金額、也可能是現金增資：對不上時不列入缺漏。
    #[test]
    fn unmatched_rights_only_event_is_not_reported() {
        let mut event = cash_event("2025-01-06", Decimal::ZERO);
        event.is_cash = false;
        event.is_stock = true;
        event.cash_dividend = None;
        let plan = build_plan(&[event], &existing(vec![]), AS_OF);
        assert!(plan.missing.is_empty());
    }

    /// 櫃買有拆分金額：同日除權息時一併修正股票股利（4175 的 1.0 → 0.9632）。
    #[test]
    fn combined_event_corrects_the_stock_amount() {
        let rows = existing(vec![row(
            1,
            2023,
            "",
            dec!(2.7452),
            dec!(1.0),
            "2023-07-20",
            "2023-07-21",
        )]);
        let event = ExDividendAnnouncement {
            is_stock: true,
            stock_dividend_ratio: Some(dec!(0.09632195232)),
            market: StockExchangeMarket::OverTheCounter,
            ..cash_event("2023-07-20", dec!(2.7452))
        };
        let plan = build_plan(&[event], &rows, AS_OF);

        let updated = plan.updates.get(&1).expect("股票股利應修正");
        assert_eq!(updated.stock_dividend, dec!(0.9632));
        assert_eq!(updated.sum, dec!(3.7084));
    }

    /// 同日除權息、資料列缺另一欄日期時補上。
    #[test]
    fn combined_event_fills_the_missing_date() {
        let rows = existing(vec![row(
            1,
            2023,
            "",
            dec!(0.2),
            dec!(0.4),
            "2024-08-26",
            "-",
        )]);
        let event = ExDividendAnnouncement {
            is_stock: true,
            stock_dividend_ratio: Some(dec!(0.04)),
            ..cash_event("2024-08-26", dec!(0.2))
        };
        let plan = build_plan(&[event], &rows, AS_OF);

        let updated = plan.updates.get(&1).expect("除權日應補上");
        assert_eq!(updated.ex_dividend_date_stock, "2024-08-26");
    }

    /// 缺漏事件從 Yahoo 補：挑同一天除權息的明細，取 Yahoo 的所屬年度與期別。
    #[test]
    fn select_fills_takes_the_yahoo_detail_of_the_same_day() {
        let mut second_h2 = row(
            10,
            2024,
            "H2",
            dec!(0.4863),
            dec!(0.214),
            "2024-09-25",
            "2024-09-25",
        );
        second_h2.year_of_dividend = 2023;
        let unrelated = row(11, 2024, "H1", dec!(0.1), Decimal::ZERO, "2024-03-01", "-");
        let event = cash_event("2024-09-25", dec!(0.4863));

        let chosen = select_fills(&[&event], &[unrelated, second_h2], &[], &HashSet::new());
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].quarter, "H2");
        assert_eq!(chosen[0].year_of_dividend, 2023);
    }

    /// 目標鍵上已有一列且已對上交易所事件時不覆蓋；同一筆 Yahoo 明細也不會補兩次。
    #[test]
    fn select_fills_never_overwrites_a_confirmed_row() {
        let confirmed = row(
            1,
            2024,
            "H2",
            dec!(0.0919),
            Decimal::ZERO,
            "2023-12-12",
            "-",
        );
        let mut yahoo_row = row(10, 2024, "H2", dec!(0.5), Decimal::ZERO, "2024-09-25", "-");
        yahoo_row.year_of_dividend = confirmed.year_of_dividend;
        let event = cash_event("2024-09-25", dec!(0.5));
        let used = HashSet::from([1]);

        assert!(
            select_fills(
                &[&event],
                std::slice::from_ref(&yahoo_row),
                std::slice::from_ref(&confirmed),
                &used
            )
            .is_empty()
        );
        // 那列沒有對上任何交易所事件（是錯的或舊的）時可以覆蓋。
        assert_eq!(
            select_fills(
                &[&event],
                &[yahoo_row.clone()],
                &[confirmed],
                &HashSet::new()
            )
            .len(),
            1
        );
        // 兩個缺漏事件指到同一筆明細時只補一次。
        let twice = select_fills(&[&event, &event], &[yahoo_row], &[], &HashSet::new());
        assert_eq!(twice.len(), 1);
    }

    /// Yahoo 也找不到同一天的明細時不補。
    #[test]
    fn select_fills_skips_events_without_a_yahoo_match() {
        let yahoo_row = row(10, 2024, "H2", dec!(0.5), Decimal::ZERO, "2024-09-26", "-");
        let event = cash_event("2024-09-25", dec!(0.5));
        assert!(select_fills(&[&event], &[yahoo_row], &[], &HashSet::new()).is_empty());
    }

    /// 證交所「權息」對上了只有現金的列：記下來，再用 Yahoo 同一除權日的配股補上（2327 國巨）。
    #[test]
    fn missing_stock_amount_is_filled_from_yahoo() {
        let rows = existing(vec![row(
            1,
            2024,
            "",
            dec!(2.0),
            Decimal::ZERO,
            "2024-08-15",
            "-",
        )]);
        let event = ExDividendAnnouncement {
            is_stock: true,
            cash_dividend: None,
            ..cash_event("2024-08-15", Decimal::ZERO)
        };
        let plan = build_plan(std::slice::from_ref(&event), &rows, AS_OF);
        assert_eq!(plan.stock_unknown.len(), 1);
        assert_eq!(plan.stock_unknown[0].0, 1);

        let yahoo = row(
            10,
            2024,
            "",
            dec!(2.0),
            dec!(1.9484),
            "2024-08-15",
            "2024-08-15",
        );
        let fixed = stock_from_yahoo(&rows["1109"][0], &event, &[yahoo]).expect("應補上配股");
        assert_eq!(fixed.stock_dividend, dec!(1.9484));
        assert_eq!(fixed.sum, dec!(3.9484));
        assert_eq!(fixed.ex_dividend_date_stock, "2024-08-15");
        assert_eq!(fixed.serial, 1);

        // Yahoo 也沒有配股時不動。
        let cash_only = row(10, 2024, "", dec!(2.0), Decimal::ZERO, "2024-08-15", "-");
        assert!(stock_from_yahoo(&rows["1109"][0], &event, &[cash_only]).is_none());
    }

    #[test]
    fn calendar_year_chunks_split_on_year_boundaries() {
        let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
        assert_eq!(
            calendar_year_chunks(d(2024, 11, 1), d(2026, 2, 3)),
            vec![
                (d(2024, 11, 1), d(2024, 12, 31)),
                (d(2025, 1, 1), d(2025, 12, 31)),
                (d(2026, 1, 1), d(2026, 2, 3)),
            ]
        );
        assert!(calendar_year_chunks(d(2026, 2, 3), d(2026, 1, 1)).is_empty());
    }
}
