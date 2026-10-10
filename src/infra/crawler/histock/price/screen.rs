//! 剔除 HiStock 排行頁前後矛盾的列。

use std::fmt;

use anyhow::Result;
use once_cell::sync::Lazy;
use rust_decimal::Decimal;

use super::HiStockFetchResult;
use crate::{core::util::daily_seen::DailySeen, infra::cache::RealtimeSnapshot};

/// 今天已用 warn 記過「列資料前後矛盾」的股票。
pub(super) static INCONSISTENT_ROW_WARNED: Lazy<std::sync::Mutex<DailySeen>> =
    Lazy::new(|| std::sync::Mutex::new(DailySeen::default()));

/// 「成交價 − 漲跌」與昨收之間允許的誤差：HiStock 漲跌到小數第二位，
/// 正常列兩者應完全相等，這裡只留一點四捨五入空間。
const CHANGE_TOLERANCE: Decimal = Decimal::from_parts(11, 0, 0, false, 3);

/// 前後矛盾的列至少要有這麼多列，才考慮整批捨棄。
const DIRTY_BATCH_MIN_ROWS: usize = 20;

/// 前後矛盾的列超過總列數的 1/N 就整批捨棄（N = 50，即 2%）。
const DIRTY_BATCH_RATIO_DIVISOR: usize = 50;

/// 排行頁太多列前後矛盾，整批不採用。
///
/// 背景迴圈遇到這個錯誤只記 warn：這是站點資料錯亂，不是程式或網路故障。
#[derive(Debug)]
pub(super) struct DirtyBatch {
    /// 前後矛盾的列數。
    pub(super) inconsistent: usize,
    /// 有效快照總數。
    pub(super) total: usize,
}

impl fmt::Display for DirtyBatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "HiStock 排行頁 {}/{} 列前後矛盾，整批捨棄",
            self.inconsistent, self.total
        )
    }
}

impl std::error::Error for DirtyBatch {}

/// 檢查單一列的欄位是否彼此吻合；吻合回傳 `None`，否則回傳原因。
///
/// HiStock 開盤後前 20 分鐘偶爾把別檔的成交價放進這一列（10-06 有 12 檔、
/// 10-08 的 1301 抓到 33 而實際 78.5）。錯得離譜的會被
/// [`Share::is_valid_price`](crate::infra::cache::Share::is_valid_price) 擋下，
/// 但錯在漲跌停範圍內的會直接寫進快取，可能誤觸高低標警報；
/// 同一列的最高最低與漲跌是獨立欄位，拿來交叉比對就能抓出這類錯列。
///
/// # 規則
/// - 尚未成交（價格為 0）不檢查，交給後續流程略過。
/// - 有最高最低時，成交價必須落在兩者之間。
/// - 有昨收時，「成交價 − 漲跌」必須等於昨收；除權息或減資恢復買賣當天，
///   漲跌以參考價為準，所以等於 `reference_price` 也算吻合。
pub(super) fn row_inconsistency(
    snapshot: &RealtimeSnapshot,
    reference_price: Option<Decimal>,
) -> Option<&'static str> {
    let price = snapshot.price;
    if price <= Decimal::ZERO {
        return None;
    }

    if snapshot.high > Decimal::ZERO
        && snapshot.low > Decimal::ZERO
        && (price < snapshot.low || price > snapshot.high)
    {
        return Some("成交價不在最高最低之間");
    }

    if snapshot.last_close > Decimal::ZERO {
        let implied_close = price - snapshot.change;
        let matches = |base: Decimal| (implied_close - base).abs() <= CHANGE_TOLERANCE;
        if !matches(snapshot.last_close) && !reference_price.is_some_and(matches) {
            return Some("成交價減漲跌不等於昨收");
        }
    }

    None
}

/// 剔除前後矛盾的列；矛盾的列太多時整批捨棄。
///
/// 剔除的列把價格設為 0，[`Share::set_stock_snapshots`](crate::infra::cache::Share::set_stock_snapshots)
/// 會改用快取裡的舊值，不會讓這檔股票從快取消失。
///
/// # 回傳
/// - `Ok(n)`：剔除了 `n` 列，其餘照常使用。
/// - `Err(DirtyBatch)`：矛盾的列超過 `max(20, 總數 2%)`，代表整頁錯亂，這一輪不更新快取。
pub(super) fn screen_inconsistent_rows(
    result: &mut HiStockFetchResult,
    reference_price: impl Fn(&str) -> Option<Decimal>,
) -> Result<usize> {
    let inconsistent: Vec<(String, &'static str)> = result
        .snapshots
        .iter()
        .filter_map(|(symbol, snapshot)| {
            row_inconsistency(snapshot, reference_price(symbol))
                .map(|reason| (symbol.clone(), reason))
        })
        .collect();

    let total = result.snapshots.len();
    let limit = DIRTY_BATCH_MIN_ROWS.max(total / DIRTY_BATCH_RATIO_DIVISOR);
    if inconsistent.len() > limit {
        return Err(DirtyBatch {
            inconsistent: inconsistent.len(),
            total,
        }
        .into());
    }

    let today = chrono::Local::now().date_naive();
    for (symbol, reason) in &inconsistent {
        if let Some(snapshot) = result.snapshots.get_mut(symbol) {
            let first = INCONSISTENT_ROW_WARNED
                .lock()
                .map(|mut seen| seen.first_today(symbol, today))
                .unwrap_or(true);
            if first {
                tracing::warn!(
                    "HiStock 列資料前後矛盾，略過: 股票 {} {}（成交 {}、漲跌 {}、昨收 {}、高 {}、低 {}；同檔今天不再重複記錄）",
                    symbol,
                    reason,
                    snapshot.price,
                    snapshot.change,
                    snapshot.last_close,
                    snapshot.high,
                    snapshot.low
                );
            } else {
                tracing::debug!("HiStock 列資料前後矛盾，略過: 股票 {} {}", symbol, reason);
            }
            snapshot.price = Decimal::ZERO;
        }
    }

    Ok(inconsistent.len())
}
