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
//!    Yahoo 也找不到的 ETF 現金配息，依配息頻率推算期別、直接用交易所日期與金額補
//!    （見 [`exchange_fill`]）；其餘只回報。
//! 5. **配股金額缺漏**：證交所「權息」拆不出股票股利，對上的資料列若配股為 0（2327 國巨
//!    2024-08-15 只記了現金），寫入模式下用 Yahoo 同一個除權日的配股金額補上。
//!
//! 一筆資料列的現金側與股票側各只會對應到一次交易所事件（除息、除權不同天時分別對上）；
//! 同日比對先全部處理完，才進行 2、3，
//! 避免一年配兩次（006208 的 7 月與 11 月）時把第二次事件誤配到第一次的資料列上。
//!
//! 證交所的「權」「權息」拆不出現金與股票股利，只用日期比對、不改金額；
//! 單純「權」也可能是現金增資，因此對不上時不列入缺漏。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};

use crate::{
    app::calculation::dividend_record,
    domain::dividend::{entity::Dividend, repository::DividendRepository},
    infra::{
        crawler::{share::ExDividendAnnouncement, tpex, twse},
        database::repository::dividend::PgDividendRepository,
    },
};

/// 交易所資料補 ETF 配息。
mod exchange_fill;
/// 比對規則（同日、日期延後、日期未公布）。
mod plan;
/// Yahoo 補缺漏。
mod yahoo_fill;

#[cfg(test)]
mod tests;

use plan::build_plan;
use yahoo_fill::fill_missing_from_yahoo;

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
    /// Yahoo 補不到、改用交易所資料補進來的 ETF 配息數（只在寫入模式）。
    pub exchange_filled: usize,
    /// 資料庫缺、Yahoo 與交易所資料都補不到的事件數（只在寫入模式）。
    pub unresolved: usize,
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
    // ETN 的配息不收錄在 dividend，留著只會每天被報成「資料庫找不到」。
    official.retain(|event| !super::is_exchange_traded_note(&event.stock_symbol));

    let repo = PgDividendRepository::new();
    // 跨年發放（12 月除息、隔年 1 月發放）的資料列在下一個發放年度，前後各多讀一年。
    let years: Vec<i32> = (start.year() - 1..=end.year() + 1).collect();
    let existing = load_existing(&repo, &years).await?;

    let plan = build_plan(&official, &existing, end);
    let mut summary = ReconcileSummary {
        official: official.len(),
        matched: plan.matched,
        updated: plan.updates.len(),
        missing: plan.missing.len(),
        ambiguous: plan.ambiguous.len(),
        filled: 0,
        stock_filled: 0,
        exchange_filled: 0,
        unresolved: 0,
    };

    if apply {
        apply_plan(&repo, &plan).await?;
        let fill = fill_missing_from_yahoo(&repo, &plan, &existing).await?;
        summary.filled = fill.filled;
        summary.stock_filled = fill.stock_filled;
        // 重新讀取，交易所補 ETF 配息時才看得到 Yahoo 剛補進來的期別。
        let existing = load_existing(&repo, &years).await?;
        let exchange = exchange_fill::fill_etf_from_exchange(
            &repo,
            &fill.unresolved,
            &official,
            &existing,
            exchange_fill::Window { start, end },
        )
        .await?;
        summary.exchange_filled = exchange.len();
        let unresolved = exchange_fill::unresolved_after_exchange_fill(&fill.unresolved, &exchange);
        summary.unresolved = unresolved.len();
        // 缺漏事件通常會在同一輪由 Yahoo 或交易所資料補上，只有補不到的才需要人處理。
        warn_missing_events(
            "交易所除權息事件在資料庫找不到、Yahoo 與交易所資料也補不到",
            &unresolved,
        );
    } else {
        let missing: Vec<String> = plan.missing.iter().map(event_label).collect();
        warn_missing_events("交易所除權息事件在資料庫找不到對應", &missing);
    }
    Ok(summary)
}

/// 讀取指定發放年度的股利資料列，依股票分組。
async fn load_existing(
    repo: &PgDividendRepository,
    years: &[i32],
) -> Result<HashMap<String, Vec<Dividend>>> {
    let mut existing: HashMap<String, Vec<Dividend>> = HashMap::new();
    for row in repo
        .fetch_by_years(years)
        .await
        .context("Failed to load dividends for reconciliation")?
    {
        existing
            .entry(row.security_code.clone())
            .or_default()
            .push(row);
    }
    Ok(existing)
}

/// 事件在日誌中的標示：`代號 除權息日`。
fn event_label(event: &ExDividendAnnouncement) -> String {
    format!("{} {}", event.stock_symbol, event.ex_date)
}

/// 有缺漏事件時記一筆 warn，最多列出 20 筆。
fn warn_missing_events(message: &str, events: &[String]) {
    if events.is_empty() {
        return;
    }
    let sample: Vec<&str> = events.iter().take(20).map(String::as_str).collect();
    tracing::warn!(count = events.len(), "{message}：{}", sample.join(", "));
}

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
