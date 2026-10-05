//! 用 Yahoo 補資料庫缺的部分：交易所有、資料庫沒有的事件整筆補進來，配股金額缺漏的列補上配股。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result};
use rust_decimal::Decimal;

use super::{DATE_FORMAT, ReconcilePlan, event_label};
use crate::{
    app::backfill::acl::YahooDividendAclMapper,
    app::calculation::dividend_record,
    domain::dividend::{entity::Dividend, repository::DividendRepository},
    infra::{
        crawler::{share::ExDividendAnnouncement, yahoo},
        database::repository::dividend::PgDividendRepository,
    },
};

/// 逐檔抓 Yahoo 的間隔，降低被 WAF 封鎖的機率。
pub(super) const YAHOO_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Yahoo 補缺漏的結果。
#[derive(Debug, Default)]
pub(super) struct YahooFill {
    /// 補入的事件數。
    pub(super) filled: usize,
    /// 補上配股的資料列數。
    pub(super) stock_filled: usize,
    /// 補不到的缺漏事件（`代號 除權息日`）。
    pub(super) unresolved: Vec<String>,
}

/// 用 Yahoo 補資料庫缺的部分：缺漏事件整筆補進來，配股金額缺漏的列補上配股。
///
/// 每檔股票只抓一次 Yahoo、間隔 2 秒；Yahoo 抓取失敗（例如已下市的 404）只記錄並略過，
/// 該檔的缺漏事件列為補不到。
pub(super) async fn fill_missing_from_yahoo(
    repo: &PgDividendRepository,
    plan: &ReconcilePlan,
    existing: &HashMap<String, Vec<Dividend>>,
) -> Result<YahooFill> {
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

    let mut fill = YahooFill::default();
    for (symbol, events) in by_symbol {
        let yahoo = match yahoo::dividend::visit(symbol).await {
            Ok(value) => value,
            Err(why) => {
                tracing::warn!("Yahoo 股利頁抓取失敗，略過補缺漏：{symbol} {why:#}");
                fill.unresolved
                    .extend(events.iter().map(|event| event_label(event)));
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
        let fills = select_fills(&events, &details, rows, &plan.used);
        fill.unresolved
            .extend(unresolved_after_fill(&events, &fills));
        for dividend in &fills {
            repo.save(dividend).await.with_context(|| {
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
            fill.stock_filled += 1;
            filled_here += 1;
        }
        fill.filled += filled_here;
        if filled_here > 0 {
            dividend_record::backfill_received_dividend_records_for_stock(symbol)
                .await
                .with_context(|| format!("Failed to rebuild dividend records for {symbol}"))?;
        }
        tokio::time::sleep(YAHOO_INTERVAL).await;
    }
    Ok(fill)
}

/// 列出補入後仍沒有對應資料列的缺漏事件（`代號 除權息日`）。
///
/// 補入列的現金或股票除權息日與事件同一天，就算該事件已補上。
pub(super) fn unresolved_after_fill(
    events: &[&ExDividendAnnouncement],
    fills: &[Dividend],
) -> Vec<String> {
    let filled_dates: BTreeSet<&str> = fills
        .iter()
        .flat_map(|dividend| {
            [
                dividend.ex_dividend_date_cash.as_str(),
                dividend.ex_dividend_date_stock.as_str(),
            ]
        })
        .collect();
    events
        .iter()
        .filter(|event| {
            !filled_dates.contains(event.ex_date.format(DATE_FORMAT).to_string().as_str())
        })
        .map(|event| event_label(event))
        .collect()
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
