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

/// 列上的昨收與快取昨收差距超過這個比例，視為別檔的列（25%）。
///
/// 單日漲跌幅上限 10%，收盤排程漏跑一天、快取昨收舊兩天也只差約 21%，不會誤判；
/// 10-28 09:21 整頁錯位的 8 檔差距都在 40% 以上。
const MISALIGNED_LAST_CLOSE_RATIO: Decimal = Decimal::from_parts(25, 0, 0, false, 2);

/// 同一批有這麼多列是別檔的列，就代表整頁錯位，整批捨棄：錯位時其他列也可能錯在漲跌停範圍內，
/// 逐列比對抓不到。
const MISALIGNED_BATCH_MIN_ROWS: usize = 3;

/// 單一列比對用的基準：快取的資料庫昨收（興櫃不比）與當日除權息參考價。
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct RowBaseline {
    /// 資料庫最後交易日收盤價；沒有資料或興櫃為 `None`（不做別檔列檢查）。
    pub(super) last_close: Option<Decimal>,
    /// 當日除權息或恢復買賣參考價。
    pub(super) reference_price: Option<Decimal>,
}

/// 整批捨棄的訊息最多附幾列樣本。
const DIRTY_BATCH_SAMPLES: usize = 3;

/// 一列前後矛盾的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Inconsistency {
    /// 成交價不在最高最低之間。
    OutOfRange,
    /// 成交價減漲跌不等於昨收（也不等於當日參考價）。
    ChangeMismatch,
    /// 列上的昨收與快取昨收差距過大，疑似別檔的列。
    Misaligned,
}

impl Inconsistency {
    /// 記錄與統計用的簡短說明。
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::OutOfRange => "成交價不在最高最低之間",
            Self::ChangeMismatch => "成交價減漲跌不等於昨收",
            Self::Misaligned => "昨收與快取昨收差距過大，疑似別檔的列",
        }
    }
}

/// 排行頁太多列前後矛盾，整批不採用。
///
/// 背景迴圈遇到這個錯誤只記 warn（開盤時段記 info，見 `report_cycle_error`）：這是站點資料錯亂，
/// 不是程式或網路故障。
#[derive(Debug, Default)]
pub(super) struct DirtyBatch {
    /// 前後矛盾的列數（含別檔的列）。
    pub(super) inconsistent: usize,
    /// 其中成交價不在最高最低之間的列數。
    pub(super) out_of_range: usize,
    /// 其中成交價減漲跌不等於昨收的列數。
    pub(super) change_mismatch: usize,
    /// 其中昨收與快取昨收差距過大、疑似別檔的列數。
    pub(super) misaligned: usize,
    /// 有效快照總數。
    pub(super) total: usize,
    /// 前幾列矛盾列的代號與欄位值（依代號排序），供事後判斷是哪種錯亂。
    pub(super) samples: Vec<String>,
}

impl fmt::Display for DirtyBatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "HiStock 排行頁 {}/{} 列前後矛盾（高低 {}、漲跌 {}、別檔 {}），整批捨棄",
            self.inconsistent, self.total, self.out_of_range, self.change_mismatch, self.misaligned
        )?;
        if !self.samples.is_empty() {
            write!(f, "；例：{}", self.samples.join("；"))?;
        }
        Ok(())
    }
}

/// 矛盾列的樣本文字：代號、原因與比對用到的欄位。
fn sample(symbol: &str, snapshot: &RealtimeSnapshot, reason: Inconsistency) -> String {
    format!(
        "{symbol} {} 價 {} 漲跌 {} 高 {} 低 {} 昨收 {}",
        reason.label(),
        snapshot.price,
        snapshot.change,
        snapshot.high,
        snapshot.low,
        snapshot.last_close
    )
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
) -> Option<Inconsistency> {
    let price = snapshot.price;
    if price <= Decimal::ZERO {
        return None;
    }

    if snapshot.high > Decimal::ZERO
        && snapshot.low > Decimal::ZERO
        && (price < snapshot.low || price > snapshot.high)
    {
        return Some(Inconsistency::OutOfRange);
    }

    if snapshot.last_close > Decimal::ZERO {
        let implied_close = price - snapshot.change;
        let matches = |base: Decimal| (implied_close - base).abs() <= CHANGE_TOLERANCE;
        if !matches(snapshot.last_close) && !reference_price.is_some_and(matches) {
            return Some(Inconsistency::ChangeMismatch);
        }
    }

    None
}

/// 這一列是否疑似別檔的資料：列上的昨收與快取昨收差距超過 [`MISALIGNED_LAST_CLOSE_RATIO`]，
/// 且也不等於當日參考價。
///
/// 整列換成別檔時成交價、漲跌、昨收、最高最低彼此吻合，[`row_inconsistency`] 抓不到；
/// 價格又可能落在本檔的漲跌停範圍內而通過 [`Share::is_valid_price`](crate::infra::cache::Share::is_valid_price)。
/// 尚未成交、列上沒有昨收或沒有快取昨收時不檢查。
pub(super) fn misaligned_row(snapshot: &RealtimeSnapshot, baseline: RowBaseline) -> bool {
    let (Some(last_close), site_close) = (baseline.last_close, snapshot.last_close) else {
        return false;
    };
    if snapshot.price <= Decimal::ZERO || site_close <= Decimal::ZERO || last_close <= Decimal::ZERO
    {
        return false;
    }
    let far = |base: Decimal| (site_close - base).abs() > base * MISALIGNED_LAST_CLOSE_RATIO;
    far(last_close) && baseline.reference_price.is_none_or(far)
}

/// 剔除前後矛盾的列；矛盾的列太多、或有 [`MISALIGNED_BATCH_MIN_ROWS`] 列以上疑似別檔時整批捨棄。
///
/// 剔除的列把價格設為 0，[`Share::set_stock_snapshots`](crate::infra::cache::Share::set_stock_snapshots)
/// 會改用快取裡的舊值，不會讓這檔股票從快取消失。
///
/// # 回傳
/// - `Ok(n)`：剔除了 `n` 列，其餘照常使用。
/// - `Err(DirtyBatch)`：矛盾的列超過 `max(20, 總數 2%)`，或疑似別檔的列達 [`MISALIGNED_BATCH_MIN_ROWS`]，
///   代表整頁錯亂，這一輪不更新快取。
pub(super) fn screen_inconsistent_rows(
    result: &mut HiStockFetchResult,
    baseline: impl Fn(&str) -> RowBaseline,
) -> Result<usize> {
    let mut inconsistent: Vec<(String, Inconsistency)> = result
        .snapshots
        .iter()
        .filter_map(|(symbol, snapshot)| {
            let baseline = baseline(symbol);
            if misaligned_row(snapshot, baseline) {
                return Some((symbol.clone(), Inconsistency::Misaligned));
            }
            row_inconsistency(snapshot, baseline.reference_price)
                .map(|reason| (symbol.clone(), reason))
        })
        .collect();
    inconsistent.sort_unstable_by(|a, b| a.0.cmp(&b.0));

    super::stats::record_inconsistent(
        inconsistent
            .iter()
            .map(|(symbol, reason)| (symbol.as_str(), *reason)),
    );
    let count = |kind: Inconsistency| inconsistent.iter().filter(|(_, r)| *r == kind).count();
    let misaligned = count(Inconsistency::Misaligned);
    let total = result.snapshots.len();
    let limit = DIRTY_BATCH_MIN_ROWS.max(total / DIRTY_BATCH_RATIO_DIVISOR);
    if inconsistent.len() > limit || misaligned >= MISALIGNED_BATCH_MIN_ROWS {
        let samples = inconsistent
            .iter()
            .take(DIRTY_BATCH_SAMPLES)
            .filter_map(|(symbol, reason)| {
                result
                    .snapshots
                    .get(symbol)
                    .map(|snapshot| sample(symbol, snapshot, *reason))
            })
            .collect();
        return Err(DirtyBatch {
            inconsistent: inconsistent.len(),
            out_of_range: count(Inconsistency::OutOfRange),
            change_mismatch: count(Inconsistency::ChangeMismatch),
            misaligned,
            total,
            samples,
        }
        .into());
    }

    for (symbol, reason) in &inconsistent {
        if let Some(snapshot) = result.snapshots.get_mut(symbol) {
            tracing::debug!(
                "HiStock 列資料前後矛盾，略過: 股票 {} {}",
                symbol,
                reason.label()
            );
            snapshot.price = Decimal::ZERO;
        }
    }

    Ok(inconsistent.len())
}
