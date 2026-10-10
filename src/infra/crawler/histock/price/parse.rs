//! HiStock 排行頁 HTML 解析。

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use once_cell::sync::Lazy;
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use super::HiStockFetchResult;
use crate::{core::util::text, infra::cache::RealtimeSnapshot};

/// 預編譯所需的選擇器
static TD_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("td").unwrap());
static ROW_SELECTOR: Lazy<Selector> = Lazy::new(|| Selector::parse("#CPHB1_gv tr").unwrap());

/// 解析單一表格列資料。
pub(super) fn parse_row(
    row: scraper::element_ref::ElementRef,
) -> Result<Option<(String, RealtimeSnapshot)>> {
    let mut tds = row.select(&TD_SELECTOR);

    // 0: 股票代號
    let symbol_node = match tds.next() {
        Some(node) => node,
        None => return Ok(None),
    };
    let symbol = symbol_node.text().collect::<String>().trim().to_string();
    if symbol.is_empty() || !symbol.chars().all(|c| c.is_ascii_digit()) {
        return Ok(None);
    }

    // 1: 股票名稱
    let name = tds
        .next()
        .context("Missing name")?
        .text()
        .collect::<String>()
        .trim()
        .to_string();

    // 輔助解析函式：嚴格處理數值解析，不再默默變 0
    let parse_val =
        |node: Option<scraper::element_ref::ElementRef>, field_name: &str| -> Result<Decimal> {
            let t = node
                .map(|n| n.text().collect::<String>())
                .unwrap_or_default();
            let t = t.trim();
            if t == "--" || t.is_empty() {
                Ok(Decimal::ZERO)
            } else {
                text::parse_decimal(t, None)
                    .map_err(|e| anyhow!("Failed to parse {} for {}: {:?}", field_name, symbol, e))
            }
        };

    let price = parse_val(tds.next(), "price")?;

    // 漲跌與幅 (處理符號與趨勢)
    let change_node = tds.next();
    let change_text = change_node
        .map(|n| n.text().collect::<String>())
        .unwrap_or_default();
    let mut change = Decimal::ZERO;
    let mut is_negative = false;
    if !change_text.contains("--") && !change_text.trim().is_empty() {
        is_negative = change_text.contains('▼');
        change = text::parse_decimal(&change_text, Some(vec!['▼', '▲', ' ', '+']))
            .map_err(|e| anyhow!("Failed to parse change for {}: {:?}", symbol, e))?;
        if is_negative && change > Decimal::ZERO {
            change = -change;
        }
    }

    let range_node = tds.next();
    let range_text = range_node
        .map(|n| n.text().collect::<String>())
        .unwrap_or_default();
    let mut change_range = Decimal::ZERO;
    if !range_text.contains("--") && !range_text.trim().is_empty() {
        change_range = text::parse_decimal(&range_text, Some(vec!['%', ' ', '+', '▼', '▲']))
            .map_err(|e| anyhow!("Failed to parse change_range for {}: {:?}", symbol, e))?;
        if is_negative && change_range > Decimal::ZERO {
            change_range = -change_range;
        }
    }

    // 跳過 5: 周漲跌, 6: 振幅
    tds.next();
    tds.next();

    let open = parse_val(tds.next(), "open")?;
    let high = parse_val(tds.next(), "high")?;
    let low = parse_val(tds.next(), "low")?;
    let last_close = parse_val(tds.next(), "last_close")?;
    let volume = parse_val(tds.next(), "volume")?;

    // 使用 new 方法強制填入必要欄位，其餘欄位則個別設定
    let mut snapshot = RealtimeSnapshot::new(symbol.clone(), price);
    snapshot.name = name;
    snapshot.source_site = "HiStock".to_string();
    snapshot.change = change;
    snapshot.change_range = change_range;
    snapshot.open = open;
    snapshot.high = high;
    snapshot.low = low;
    snapshot.last_close = last_close;
    snapshot.volume = volume;

    Ok(Some((symbol, snapshot)))
}

/// 解析 HiStock 排行榜頁面的 HTML，轉成全市場即時快照。
///
/// 這是一個「純函式」——輸入只有 HTML 字串，不做任何網路 I/O。
/// 之所以把它從 [`fetch_all_from_rank`](super::fetch_all_from_rank) 拆出來，是為了讓單元測試能用
/// `testdata/rank_page.html` fixture 直接驗證整頁解析流程
/// （選擇器命中、表頭略過、逐列解析），而不需要真的連 HiStock。
///
/// # 解析規則
/// - 只掃 `#CPHB1_gv tr`（HiStock 排行榜的表格 id）。
/// - 表頭列與代號非純數字的列由 [`parse_row`] 回傳 `None` 略過。
/// - 任何一列的數值欄位損壞會讓整頁解析失敗（嚴格模式）——
///   寧可這一輪不更新快取，也不要把壞資料混進即時報價。
///
/// # 回傳
/// - `Ok(HiStockFetchResult)`：快照 map、原始 body 大小與掃描列數（供診斷）。
/// - `Err(_)`：HTML 結構異常、數值損壞，或解析後完全沒有有效資料。
pub(super) fn parse_rank_html(body: &str) -> Result<HiStockFetchResult> {
    let body_bytes = body.len();
    let document = Html::parse_document(body);

    let mut map = HashMap::with_capacity(1200);
    let mut row_count = 0usize;
    for row in document.select(&ROW_SELECTOR) {
        row_count += 1;
        if let Some((symbol, snapshot)) = parse_row(row)? {
            map.insert(symbol, snapshot);
        }
    }

    if map.is_empty() {
        return Err(anyhow!("Failed to parse HiStock rank page (empty map)"));
    }
    Ok(HiStockFetchResult {
        snapshots: map,
        body_bytes,
        row_count,
    })
}
