use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use chrono::{Months, NaiveDate};
use rust_decimal::Decimal;

use crate::domain::performance::{
    CagrPeriod, CagrRepository, CagrSourceRepository, DividendEvent, StockCagr,
    entity::{BASE_DATE_GRACE_DAYS, PRINCIPAL},
    simulator::{SimulationInput, simulate},
};

/// 單次批次寫入的列數上限。
///
/// 全市場約 2,600 檔 × 8 個期間 ≈ 20,700 列，分批送避免單一 statement 過大。
const SAVE_CHUNK_SIZE: usize = 2_000;

/// 每日 CAGR 計算的執行摘要。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CagrCalculationSummary {
    /// 計算基準日（期末交易日）。
    pub date: Option<NaiveDate>,
    /// 母體檔數。
    pub universe: usize,
    /// 實際計算的期間數（扣除因資料不足而整個跳過者）。
    pub periods_calculated: usize,
    /// 因資料庫歷史深度不足而整個跳過的期間數。
    pub periods_skipped: usize,
    /// 寫入的資料列數。
    pub rows_written: u64,
    /// 標記為疑似異常的股票檔數。
    pub anomaly_symbols: usize,
}

/// 排程進入點：以資料庫中最新交易日為基準計算全市場 CAGR。
///
/// 刻意排在每日 05:40（台北時間）—— 也就是 21:00 的年度配息回補與 05:00–05:30
/// 各項回補之後。CAGR 完全依賴股利資料，若在收盤鏈（15:00）就算完，當晚回補
/// 進來的股利不會反映到當日結果。
pub async fn execute_scheduled() -> Result<()> {
    let summary = execute(None).await?;
    match summary.date {
        Some(date) => tracing::info!(
            date = %date,
            universe = summary.universe,
            periods_calculated = summary.periods_calculated,
            periods_skipped = summary.periods_skipped,
            rows_written = summary.rows_written,
            anomaly_symbols = summary.anomaly_symbols,
            "每日 CAGR 計算完成"
        ),
        None => tracing::warn!("每日 CAGR 計算未產生任何結果（無可用交易日或母體為空）"),
    }
    Ok(())
}

/// 計算並寫入指定基準日的全市場 CAGR。
///
/// 傳入 `None` 時自動採用資料庫中最新的交易日。
pub async fn execute(date: Option<NaiveDate>) -> Result<CagrCalculationSummary> {
    let source = crate::infra::database::repository::cagr_source::PgCagrSourceRepository::new();
    let repository = crate::infra::database::repository::performance::PgCagrRepository::new();
    calculate(&source, &repository, date).await
}

/// 實際的計算流程。
///
/// 以 trait 物件而非具體型別接收相依，讓流程本身可以在測試中注入假資料源驗證，
/// 不需要真實資料庫。
pub async fn calculate(
    source: &dyn CagrSourceRepository,
    repository: &dyn CagrRepository,
    date: Option<NaiveDate>,
) -> Result<CagrCalculationSummary> {
    let mut summary = CagrCalculationSummary::default();

    // ── 1. 決定期末交易日 ────────────────────────────────────────────
    let end_date = match date {
        Some(d) => source
            .fetch_trading_day_on_or_before(d)
            .await
            .context("Failed to align end date to a trading day")?,
        None => source
            .fetch_latest_trading_day()
            .await
            .context("Failed to fetch the latest trading day")?,
    };
    let Some(end_date) = end_date else {
        tracing::warn!("報價資料中找不到任何交易日，略過本次 CAGR 計算");
        return Ok(summary);
    };
    summary.date = Some(end_date);

    // ── 2. 母體與期末價格 ────────────────────────────────────────────
    let symbols = source
        .fetch_active_symbols()
        .await
        .context("Failed to fetch active symbols")?;
    summary.universe = symbols.len();
    if symbols.is_empty() {
        tracing::warn!("計算母體為空，略過本次 CAGR 計算");
        return Ok(summary);
    }

    let end_prices: HashMap<String, Decimal> = source
        .fetch_closing_prices_on(end_date)
        .await
        .context("Failed to fetch closing prices on the end date")?
        .into_iter()
        .collect();

    let first_quote_dates: HashMap<String, NaiveDate> = source
        .fetch_first_quote_dates()
        .await
        .context("Failed to fetch first quote dates")?
        .into_iter()
        .collect();

    // ── 3. 逐期間對齊期初交易日 ──────────────────────────────────────
    //
    // 採「全市場統一對齊」：先算出整個市場共同的期初交易日，個股當天沒有報價
    // 時才套用寬限規則。若改成每檔各自往前找，長期停牌的股票會取到很久以前的
    // 價格，「期間」名不符實，也無法橫向比較。
    let mut aligned: Vec<(CagrPeriod, NaiveDate, NaiveDate)> = Vec::new();
    for period in CagrPeriod::ALL {
        let Some(target) = end_date.checked_sub_months(Months::new(period.months())) else {
            continue;
        };
        match source
            .fetch_trading_day_on_or_before(target)
            .await
            .with_context(|| format!("Failed to align base date for period {}", period.code()))?
        {
            Some(base_date) => aligned.push((period, target, base_date)),
            None => {
                // 資料庫的歷史深度不足以涵蓋此期間，整個期間跳過而非寫入
                // 兩萬多列全 NULL 的資料。
                tracing::warn!(
                    period = period.code(),
                    target = %target,
                    "報價歷史不足以涵蓋此期間，整個跳過"
                );
                summary.periods_skipped += 1;
            }
        }
    }
    if aligned.is_empty() {
        tracing::warn!("沒有任何期間的期初交易日可對齊，略過本次 CAGR 計算");
        return Ok(summary);
    }
    summary.periods_calculated = aligned.len();

    // 最長期間的期初日，用於一次撈齊股利事件與異常偵測範圍。
    let earliest_base_date = aligned
        .iter()
        .map(|(_, _, base_date)| *base_date)
        .min()
        .unwrap_or(end_date);

    // ── 4. 股利事件（一次撈齊最長期間，於記憶體分派到各期間）──────────
    //
    // 八個期間中最長者涵蓋其餘所有期間，只掃一次即可；對 dividend 表掃八輪
    // 是不必要的浪費。
    let events = source
        .fetch_dividend_events_since(earliest_base_date)
        .await
        .context("Failed to fetch dividend events")?;
    let events_by_symbol = group_events_by_symbol(events);

    // ── 5. 再投入口徑所需的除息日價格（批次，不逐筆查詢）──────────────
    let reinvest_pairs = collect_reinvest_pairs(&events_by_symbol, earliest_base_date, end_date);
    let reinvest_prices = source
        .fetch_closing_prices_at(&reinvest_pairs)
        .await
        .context("Failed to fetch closing prices for reinvestment")?;
    let reinvest_prices = group_prices_by_symbol(reinvest_prices);

    // ── 6. 異常偵測（疑似減資／分割）────────────────────────────────
    //
    // 只查一次最長期間的事件，再依日期切分到各期間。若改成「用最長期間算一次
    // 代號清單、套用到所有期間」，2017 年的減資會把 3 個月期間的結果也標記為
    // 異常 —— 那個旗標的意思是「這個期間內發生過未建模的公司行動」，跨期間共用
    // 語義就錯了。
    let anomaly_events = source
        .fetch_anomaly_events(earliest_base_date, end_date)
        .await
        .context("Failed to detect anomaly events")?;
    summary.anomaly_symbols = anomaly_events
        .iter()
        .map(|(symbol, _)| symbol.as_str())
        .collect::<HashSet<_>>()
        .len();

    // ── 7. 逐期間計算 ───────────────────────────────────────────────
    let empty_events: Vec<DividendEvent> = Vec::new();
    let empty_prices: HashMap<NaiveDate, Decimal> = HashMap::new();

    for (period, target, base_date) in aligned {
        // 只採計落在本期間 (base_date, end_date] 內的異常事件。
        let period_anomalies: HashSet<&str> = anomaly_events
            .iter()
            .filter(|(_, event_date)| *event_date > base_date && *event_date <= end_date)
            .map(|(symbol, _)| symbol.as_str())
            .collect();

        let base_prices: HashMap<String, Decimal> = source
            .fetch_closing_prices_on(base_date)
            .await
            .with_context(|| {
                format!(
                    "Failed to fetch closing prices for period {}",
                    period.code()
                )
            })?
            .into_iter()
            .collect();

        // 寬限查詢：期初交易日當天沒有報價的股票，往後找寬限期內的第一筆。
        // 這讓「期初日剛好停牌三天的老股票」仍能計算，同時把「上個月才上市的
        // 新股」明確擋在門檻外 —— 兩者的資訊價值天差地遠，不該同等對待。
        let grace_deadline = base_date
            .checked_add_signed(chrono::Duration::days(BASE_DATE_GRACE_DAYS))
            .unwrap_or(base_date);
        let grace_quotes: HashMap<String, (NaiveDate, Decimal)> = source
            .fetch_first_quote_within(base_date, grace_deadline)
            .await
            .with_context(|| format!("Failed to fetch grace-period quotes for {}", period.code()))?
            .into_iter()
            .map(|(symbol, quote_date, price)| (symbol, (quote_date, price)))
            .collect();

        let mut records = Vec::with_capacity(symbols.len());
        for symbol in &symbols {
            let events = events_by_symbol.get(symbol).unwrap_or(&empty_events);
            let prices = reinvest_prices.get(symbol).unwrap_or(&empty_prices);
            records.push(build_record(
                symbol,
                period,
                end_date,
                target,
                base_date,
                base_prices.get(symbol).copied(),
                grace_quotes.get(symbol).copied(),
                end_prices.get(symbol).copied(),
                first_quote_dates.get(symbol).copied(),
                events,
                prices,
                period_anomalies.contains(symbol.as_str()),
            ));
        }

        for chunk in records.chunks(SAVE_CHUNK_SIZE) {
            summary.rows_written += repository
                .save_batch(chunk)
                .await
                .with_context(|| format!("Failed to save CAGR batch for {}", period.code()))?;
        }

        tracing::info!(
            period = period.code(),
            base_date = %base_date,
            rows = records.len(),
            "CAGR 期間計算完成"
        );
    }

    Ok(summary)
}

/// 組出單一股票在單一期間的計算結果。
///
/// 資料不足時回傳的 `StockCagr` 各數值欄位為 `None`，`data_complete` 為
/// `false` —— 刻意不寫 0，否則「上市半年的新股」會在十年排行榜上被誤讀成
/// 「十年零報酬」。這類列仍然寫入資料庫，讓「某檔在某期間的狀態」永遠查得到，
/// API 才能穩定回傳、前端也不必區分「查無資料」與「不可計算」兩套邏輯。
#[allow(clippy::too_many_arguments)]
fn build_record(
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

/// 將股利事件依股票代號分組。
fn group_events_by_symbol(events: Vec<DividendEvent>) -> HashMap<String, Vec<DividendEvent>> {
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
fn collect_reinvest_pairs(
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
fn group_prices_by_symbol(
    prices: Vec<(String, NaiveDate, Decimal)>,
) -> HashMap<String, HashMap<NaiveDate, Decimal>> {
    let mut grouped: HashMap<String, HashMap<NaiveDate, Decimal>> = HashMap::new();
    for (symbol, date, price) in prices {
        grouped.entry(symbol).or_default().insert(date, price);
    }
    grouped
}
