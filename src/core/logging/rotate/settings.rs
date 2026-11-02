//! 日誌輪替的全域設定：單檔大小上限與保留天數的預設值、可設定範圍與啟動時套用。

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// 預設單檔最大大小：10 MB
pub(super) const DEFAULT_MAX_SIZE: u64 = 10 * 1024 * 1024;
/// 預設保留天數：7 天
pub(super) const DEFAULT_MAX_AGE_DAYS: i64 = 7;
/// 同一秒內輪轉檔名的最大序號，避免極端情況下無限探測檔名。
pub(super) const MAX_FILENAME_SEQ: u32 = 1_000;
/// 單檔大小可設定範圍（bytes）：1 MB ～ 1 GB。
pub(super) const MIN_CONFIGURABLE_SIZE: u64 = 1024 * 1024;
pub(super) const MAX_CONFIGURABLE_SIZE: u64 = 1024 * 1024 * 1024;
/// 保留天數可設定範圍：1 ～ 365 天。
pub(super) const MIN_CONFIGURABLE_AGE_DAYS: i64 = 1;
pub(super) const MAX_CONFIGURABLE_AGE_DAYS: i64 = 365;

/// 全域單檔大小上限（bytes），供未指定固定值的 [`Rotate`](super::Rotate) 共用。
pub(super) static GLOBAL_MAX_SIZE: AtomicU64 = AtomicU64::new(DEFAULT_MAX_SIZE);
/// 全域日誌保留天數，供未指定固定值的 [`Rotate`](super::Rotate) 共用。
pub(super) static GLOBAL_MAX_AGE_DAYS: AtomicI64 = AtomicI64::new(DEFAULT_MAX_AGE_DAYS);

/// 套用外部設定（`app.json` 的 `logging.file` 或 `.env`）到輪轉參數。
///
/// 由 `main` 在設定載入後呼叫。logger 的背景 worker 在此之前就已建立
/// `Rotate`，因此參數以全域 atomic 保存並在每次判斷時讀取，讓設定即時生效，
/// 而不需重建 writer（也避免 logger 初始化時反向依賴 `SETTINGS` 造成死鎖）。
///
/// 兩個參數為 `0` 時代表未設定，維持既有值；非 0 值會收斂到安全範圍內
/// （大小 1 MB～1 GB、天數 1～365）。
pub fn configure(max_size_mb: u64, max_age_days: i64) {
    if max_size_mb > 0 {
        let bytes = max_size_mb
            .saturating_mul(1024 * 1024)
            .clamp(MIN_CONFIGURABLE_SIZE, MAX_CONFIGURABLE_SIZE);
        GLOBAL_MAX_SIZE.store(bytes, Ordering::Relaxed);
    }

    if max_age_days > 0 {
        GLOBAL_MAX_AGE_DAYS.store(
            max_age_days.clamp(MIN_CONFIGURABLE_AGE_DAYS, MAX_CONFIGURABLE_AGE_DAYS),
            Ordering::Relaxed,
        );
    }
}

/// 取得目前生效的單檔大小上限（bytes）與保留天數，供啟動時記錄與診斷。
pub fn effective_settings() -> (u64, i64) {
    (
        GLOBAL_MAX_SIZE.load(Ordering::Relaxed),
        GLOBAL_MAX_AGE_DAYS.load(Ordering::Relaxed),
    )
}
