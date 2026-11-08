//! # 異常價格過濾的紀錄與收盤統計
//!
//! 採集到的成交價超出漲跌幅（興櫃為倍數區間）時，驗價會把它擋下並呼叫
//! [`report_abnormal_price`]。這裡只負責記錄：每個站點每天第一筆記 WARN 當範例，
//! 其餘記 DEBUG；收盤時由 [`flush_abnormal_price_stats`] 依站點輸出當天被擋次數與檔數，每檔附第一次
//! 被擋的採集價與昨收，事後不用翻 DEBUG 也看得出錯成什麼價。
//!
//! 2026-10-27～30 共 12 筆全是 HiStock 排行頁個別股票的錯價，例如 3189 採到 452.96
//! （不合 0.5 元跳動單位，當天實際成交 886～946）、3481 採到 1.59（實際 41.5～43.6），
//! 屬來源本身的錯，驗價已擋下；價格追蹤讀的是驗證後的快取，不會因此誤發通知。

use std::{collections::BTreeMap, sync::Mutex};

use chrono::{Local, NaiveDate};
use once_cell::sync::Lazy;
use rust_decimal::Decimal;

/// 收盤統計每站最多列出的股票代號數，其餘只算檔數。
const MAX_LISTED_SYMBOLS: usize = 10;

/// 沒有標示來源站點時的名稱。
const UNKNOWN_SITE: &str = "未標示來源";

/// 當天各站點被擋下的異常價格。
static STATS: Lazy<Mutex<AbnormalPriceStats>> = Lazy::new(Mutex::default);

/// 一檔股票當天第一次被擋時的採集價與昨收。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlockedPrice {
    price: Decimal,
    last_close: Decimal,
}

/// 單一站點當天的被擋次數與股票（代號對應第一次被擋的價格）。
#[derive(Debug, Default)]
struct SiteCount {
    count: usize,
    symbols: BTreeMap<String, BlockedPrice>,
}

/// 依站點累計的異常價格；換日時自動清空。
#[derive(Debug, Default)]
struct AbnormalPriceStats {
    day: Option<NaiveDate>,
    sites: BTreeMap<String, SiteCount>,
}

impl AbnormalPriceStats {
    /// 記一筆被擋的價格，回傳是否為該站當天第一筆；同一檔只留第一次被擋的價格。
    fn record(
        &mut self,
        today: NaiveDate,
        site: &str,
        symbol: &str,
        blocked: BlockedPrice,
    ) -> bool {
        if self.day != Some(today) {
            self.day = Some(today);
            self.sites.clear();
        }
        let entry = self.sites.entry(site.to_string()).or_default();
        entry.count += 1;
        entry.symbols.entry(symbol.to_string()).or_insert(blocked);
        entry.count == 1
    }

    /// 收盤統計行；當天沒有被擋的價格時為 `None`。
    ///
    /// 格式例如 `今日過濾異常價格：HiStock 4 次（3 檔：3189 452.96／昨收 900、…）`；週報
    /// （`data_health::abnormal_prices`）以 `；` 分站、以 ` 次（` 取次數，代號後的價格不能含這兩個字串。
    fn summary(&self) -> Option<String> {
        if self.sites.is_empty() {
            return None;
        }
        let parts: Vec<String> = self
            .sites
            .iter()
            .map(|(site, count)| {
                let listed: Vec<String> = count
                    .symbols
                    .iter()
                    .take(MAX_LISTED_SYMBOLS)
                    .map(|(symbol, blocked)| {
                        format!(
                            "{symbol} {}／昨收 {}",
                            blocked.price.normalize(),
                            blocked.last_close.normalize()
                        )
                    })
                    .collect();
                let more = if count.symbols.len() > MAX_LISTED_SYMBOLS {
                    "…"
                } else {
                    ""
                };
                format!(
                    "{site} {} 次（{} 檔：{}{more}）",
                    count.count,
                    count.symbols.len(),
                    listed.join("、")
                )
            })
            .collect();
        Some(format!("今日過濾異常價格：{}", parts.join("；")))
    }
}

/// 記錄一筆被過濾的異常價格：同一站點每天第一筆記 WARN，其餘記 DEBUG，次數都計入收盤統計。
///
/// 站點出錯時常一次錯好幾檔（10-28 HiStock 8 檔），逐檔 WARN 只會淹沒其他警告；
/// 哪些股票被擋改看收盤的 [`flush_abnormal_price_stats`]。
pub fn report_abnormal_price(symbol: &str, price: Decimal, last_close: Decimal, site: &str) {
    let site = site_label(site);
    // 鎖中毒時不讀舊狀態，照常記 WARN，寧可多記也不漏記。
    let first = STATS
        .lock()
        .map(|mut stats| {
            let blocked = BlockedPrice { price, last_close };
            stats.record(Local::now().date_naive(), site, symbol, blocked)
        })
        .unwrap_or(true);
    if first {
        tracing::warn!(
            "過濾異常價格！股票: {symbol}, 採集價格: {price}, 昨收價: {last_close}, 站點: {site}（同站今天之後的記 DEBUG，次數見收盤統計）"
        );
    } else {
        tracing::debug!(
            "過濾異常價格！股票: {symbol}, 採集價格: {price}, 昨收價: {last_close}, 站點: {site}"
        );
    }
}

/// 統計與日誌用的站點名稱：沒有標示來源時為「未標示來源」。
fn site_label(site: &str) -> &str {
    if site.is_empty() { UNKNOWN_SITE } else { site }
}

/// 收盤時輸出當天各站點被擋的異常價格次數與檔數，並清空統計；沒有被擋的價格時不輸出。
pub fn flush_abnormal_price_stats() {
    let summary = match STATS.lock() {
        Ok(mut stats) => {
            let line = stats.summary();
            stats.sites.clear();
            line
        }
        Err(_) => {
            tracing::error!("異常價格統計：狀態鎖中毒，略過收盤統計");
            return;
        }
    };
    if let Some(line) = summary {
        tracing::info!("{line}");
    }
}

#[cfg(test)]
mod tests;
