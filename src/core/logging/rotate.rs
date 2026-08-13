use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
    },
    time::UNIX_EPOCH,
};

use anyhow::Result;
use chrono::{DateTime, Local, TimeDelta};
use rayon::prelude::*;

use crate::core::logging;

/// 預設單檔最大大小：10 MB
const DEFAULT_MAX_SIZE: u64 = 10 * 1024 * 1024;
/// 預設保留天數：7 天
const DEFAULT_MAX_AGE_DAYS: i64 = 7;
/// 同一秒內輪轉檔名的最大序號，避免極端情況下無限探測檔名。
const MAX_FILENAME_SEQ: u32 = 1_000;
/// 單檔大小可設定範圍（bytes）：1 MB ～ 1 GB。
const MIN_CONFIGURABLE_SIZE: u64 = 1024 * 1024;
const MAX_CONFIGURABLE_SIZE: u64 = 1024 * 1024 * 1024;
/// 保留天數可設定範圍：1 ～ 365 天。
const MIN_CONFIGURABLE_AGE_DAYS: i64 = 1;
const MAX_CONFIGURABLE_AGE_DAYS: i64 = 365;

/// 全域單檔大小上限（bytes），供未指定固定值的 [`Rotate`] 共用。
static GLOBAL_MAX_SIZE: AtomicU64 = AtomicU64::new(DEFAULT_MAX_SIZE);
/// 全域日誌保留天數，供未指定固定值的 [`Rotate`] 共用。
static GLOBAL_MAX_AGE_DAYS: AtomicI64 = AtomicI64::new(DEFAULT_MAX_AGE_DAYS);

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

/// 依日期與大小自動輪轉的檔案 writer。
pub struct Rotate {
    /// 檔名模式，例如 "log/%Y-%m-%d-name.log"
    fn_pattern: String,
    /// 當前完整檔名（含開檔時間戳）
    cur_fn: String,
    /// 當前完整檔名的同步保護鎖。
    cur_fn_lock: RwLock<String>,
    /// 當前基礎檔名（不含時間戳，由日期決定；僅用於偵測跨日）
    cur_base_fn: String,
    /// 檔案輸出 handle
    out_fh: Option<Arc<RwLock<BufWriter<File>>>>,
    /// 當日已輪轉次數，僅供診斷與測試觀察（檔名改以時間戳區分）
    generation: u32,
    /// 單檔最大大小 (bytes)；`None` 表示跟隨全域設定
    max_size: Option<u64>,
    /// 當前檔案已寫入大小
    current_size: u64,
    /// 日誌保留天數；`None` 表示跟隨全域設定
    max_age_days: Option<i64>,
    /// 是否正在執行輪轉
    on_rotate: AtomicBool,
}

impl Rotate {
    /// 建立跟隨全域設定的 Rotate 實例。
    ///
    /// 單檔大小與保留天數取自 [`configure`] 套用的值（預設 10 MB / 7 天），
    /// 且每次判斷時重新讀取，設定載入後不必重建 writer 即可生效。
    pub fn new(fn_pattern: String) -> Self {
        Self::build(fn_pattern, None, None)
    }

    /// 使用自訂設定建立 Rotate 實例（不受全域設定影響）。
    ///
    /// # Arguments
    /// * `fn_pattern` - 檔名模式，例如 "log/%Y-%m-%d-app.log"
    /// * `max_size` - 單檔最大大小 (bytes)
    /// * `max_age_days` - 日誌保留天數
    pub fn with_options(fn_pattern: String, max_size: u64, max_age_days: i64) -> Self {
        Self::build(fn_pattern, Some(max_size), Some(max_age_days))
    }

    /// 共用建構流程。
    fn build(fn_pattern: String, max_size: Option<u64>, max_age_days: Option<i64>) -> Self {
        Rotate {
            fn_pattern,
            cur_fn: String::new(),
            cur_fn_lock: Default::default(),
            cur_base_fn: String::new(),
            out_fh: None,
            generation: 0,
            max_size,
            current_size: 0,
            max_age_days,
            on_rotate: Default::default(),
        }
    }

    /// 取得目前生效的單檔大小上限 (bytes)。
    fn max_size(&self) -> u64 {
        self.max_size
            .unwrap_or_else(|| GLOBAL_MAX_SIZE.load(Ordering::Relaxed))
    }

    /// 取得目前生效的日誌保留時間。
    fn max_age(&self) -> chrono::Duration {
        let days = self
            .max_age_days
            .unwrap_or_else(|| GLOBAL_MAX_AGE_DAYS.load(Ordering::Relaxed));

        TimeDelta::try_days(days).unwrap_or_else(|| TimeDelta::days(DEFAULT_MAX_AGE_DAYS))
    }

    /// 確保當前有可用的檔案寫入器，必要時因跨日而切換新檔。
    fn ensure_writer(&mut self, now: DateTime<Local>) -> Result<()> {
        let base_fn = self.generate_base_fn(now);

        // 日期未變更且檔案已開啟：沿用現有 writer
        if base_fn == self.cur_base_fn && self.out_fh.is_some() {
            return Ok(());
        }

        // 跨日（或首次開檔）：重設輪轉計數並開新檔
        self.generation = 0;
        self.current_size = 0;
        self.cur_base_fn = base_fn;
        self.open_new_file(now)?;
        self.cleanup_old_files(now);

        Ok(())
    }

    /// 寫入日誌訊息，自動處理跨日切檔與大小超限輪轉。
    ///
    /// 寫入後立即 flush，確保日誌即時落地（程序異常結束時不遺失緩衝內容）。
    ///
    /// # Arguments
    /// * `now` - 當前時間，同時作為輪轉後新檔名的時間戳
    /// * `msg` - 要寫入的訊息
    pub fn write_msg(&mut self, now: DateTime<Local>, msg: &[u8]) -> Result<()> {
        // 確保有有效的 writer（含跨日切檔）
        self.ensure_writer(now)?;

        // 檢查是否需要因大小超限而輪轉
        if self.should_rotate_by_size(msg.len()) {
            self.rotate_by_size(now)?;
        }

        let writer = self
            .out_fh
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Failed to get writer"))?;
        let mut w = writer
            .write()
            .map_err(|_| anyhow::anyhow!("log writer lock poisoned"))?;

        w.write_all(msg)?;
        w.flush()?;
        self.current_size += msg.len() as u64;

        Ok(())
    }

    /// 產生基礎檔名（根據日期，不含時間戳）
    fn generate_base_fn(&self, now: DateTime<Local>) -> String {
        now.format(&self.fn_pattern).to_string()
    }

    /// 產生完整檔名（含開檔時間戳）
    ///
    /// 時間戳為該檔第一筆日誌的時間（時-分-秒），排查時可直接由檔名定位時段。
    /// Windows 檔名不允許 `:`，因此改用 `-` 分隔。
    ///
    /// seq = 0: "log/2025-02-03-app.00-05-32.log"
    /// seq = 1: "log/2025-02-03-app.00-05-32-1.log"（同一秒內再次輪轉時避免撞名）
    fn generate_full_fn(&self, base_fn: &str, stamp: &str, seq: u32) -> String {
        let path = Path::new(base_fn);
        let parent = path.parent().unwrap_or(Path::new(""));
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("log");
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("log");
        let suffix = if seq == 0 {
            stamp.to_string()
        } else {
            format!("{}-{}", stamp, seq)
        };

        parent
            .join(format!("{}.{}.{}", stem, suffix, ext))
            .to_string_lossy()
            .to_string()
    }

    /// 依開檔時間挑出尚未被使用的檔名。
    ///
    /// 同一秒內多次輪轉（或重啟）時遞增序號，確保不會接續寫入既有檔案。
    fn resolve_available_fn(&self, now: DateTime<Local>) -> String {
        let stamp = now.format("%H-%M-%S").to_string();
        let mut seq = 0;

        loop {
            let candidate = self.generate_full_fn(&self.cur_base_fn, &stamp, seq);
            if !Path::new(&candidate).exists() || seq >= MAX_FILENAME_SEQ {
                return candidate;
            }

            seq += 1;
        }
    }

    /// 檢查是否需要因大小超限而輪轉
    ///
    /// 空檔案不輪轉，避免單筆訊息大於 `max_size` 時每次寫入都開新檔。
    fn should_rotate_by_size(&self, additional_bytes: usize) -> bool {
        self.current_size > 0 && self.current_size + additional_bytes as u64 > self.max_size()
    }

    /// 開啟新檔案，檔名帶當下時間戳。
    fn open_new_file(&mut self, now: DateTime<Local>) -> Result<()> {
        // 先 flush 並關閉舊檔案
        self.flush_current();

        // 確保目錄存在（須早於檔名可用性檢查，否則 exists() 永遠為 false）
        if let Some(parent) = Path::new(&self.cur_base_fn).parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }

        let filename = self.resolve_available_fn(now);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&filename)?;

        // 取得現有檔案大小（撞名上限時可能接續既有檔）
        self.current_size = file.metadata().map(|m| m.len()).unwrap_or(0);

        self.out_fh = Some(Arc::new(RwLock::new(BufWriter::with_capacity(4096, file))));
        self.cur_fn = filename;

        if let Ok(mut lock) = self.cur_fn_lock.write() {
            lock.clone_from(&self.cur_fn);
        }

        Ok(())
    }

    /// 執行輪轉（因大小超限），新檔以當下時間為檔名時間戳。
    fn rotate_by_size(&mut self, now: DateTime<Local>) -> Result<()> {
        self.generation += 1;
        self.current_size = 0;

        self.open_new_file(now)
    }

    /// flush 當前檔案
    fn flush_current(&self) {
        if let Some(ref writer) = self.out_fh
            && let Ok(mut w) = writer.write()
        {
            let _ = w.flush();
        }
    }

    /// 清理舊檔案（超過 max_age 的檔案）
    fn cleanup_old_files(&self, now: DateTime<Local>) {
        if self.on_rotate.swap(true, Ordering::Relaxed) {
            return;
        }

        match Self::files_in_directory(&self.cur_fn) {
            Ok(files) => {
                let cut_off = (now - self.max_age()).timestamp() as u64;
                let to_unlink: Vec<PathBuf> = files
                    .into_iter()
                    .filter_map(|file| {
                        fs::metadata(&file)
                            .and_then(|metadata| metadata.modified())
                            .map(|system_time| system_time.duration_since(UNIX_EPOCH))
                            .ok()
                            .filter(|file_duration| match file_duration {
                                Ok(duration) => duration.as_secs() <= cut_off,
                                Err(_) => false,
                            })
                            .map(|_| file)
                    })
                    .collect();

                if !to_unlink.is_empty() {
                    to_unlink
                        .par_iter()
                        .with_min_len(num_cpus::get())
                        .for_each(|unlink| match fs::remove_file(unlink) {
                            Err(why) => {
                                logging::error_console(format!(
                                    "couldn't remove the file({}). because {:?}",
                                    unlink.display(),
                                    why
                                ));
                            }
                            Ok(_) => {
                                logging::info_file_async(format!(
                                    "the file has been deleted:{}",
                                    unlink.display()
                                ));
                            }
                        });
                }
            }
            Err(why) => {
                logging::error_console(format!(
                    "Failed to list_files_in_directory because {:?}",
                    why
                ));
            }
        }

        self.on_rotate.store(false, Ordering::Relaxed);
    }

    /// 列出指定檔案所在目錄的所有檔案。
    fn files_in_directory<P: AsRef<Path>>(file_path: P) -> Result<Vec<PathBuf>, io::Error> {
        let path = file_path.as_ref();
        let parent_dir = path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Parent directory not found"))?;

        let mut files = Vec::new();
        for entry in fs::read_dir(parent_dir)? {
            let entry = entry?;
            files.push(entry.path());
        }

        Ok(files)
    }
}

impl Drop for Rotate {
    fn drop(&mut self) {
        self.flush_current();
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use crate::core::logging;

    use super::*;

    /// 驗證基本寫檔流程。
    #[tokio::test]
    #[ignore]
    async fn test_basic_write() {
        dotenvy::dotenv().ok();
        logging::debug_file_async("開始 test_basic_write".to_string());

        let mut r = Rotate::new("log/%Y-%m-%d-test.log".to_string());
        let now = Local::now();

        // 使用新的 write_msg 方法
        let msg = format!("{} 測試訊息\r\n", now.format("%F %X%.6f"));
        if let Err(why) = r.write_msg(now, msg.as_bytes()) {
            logging::error_console(format!("Failed to write: {:?}", why));
        }

        logging::debug_file_async("結束 test_basic_write".to_string());
    }

    /// 驗證跨日期時會切換新檔案。
    #[tokio::test]
    #[ignore]
    async fn test_date_rotation() {
        dotenvy::dotenv().ok();
        logging::debug_file_async("開始 test_date_rotation".to_string());

        let mut r = Rotate::new("log/%Y-%m-%d-test.log".to_string());
        let mut now = Local::now();

        // 第一天寫入
        let msg1 = format!("{} Day 1\r\n", now.format("%F %X%.6f"));
        let _ = r.write_msg(now, msg1.as_bytes());

        // 模擬下一天
        now += TimeDelta::try_days(1).unwrap();
        let msg2 = format!("{} Day 2\r\n", now.format("%F %X%.6f"));
        let _ = r.write_msg(now, msg2.as_bytes());

        logging::debug_file_async("結束 test_date_rotation".to_string());
    }

    /// 驗證超過檔案大小限制時會輪轉。
    #[tokio::test]
    #[ignore]
    async fn test_size_rotation() {
        dotenvy::dotenv().ok();
        logging::debug_file_async("開始 test_size_rotation".to_string());

        // 設定很小的檔案大小限制 (1KB) 來測試輪轉
        let mut r = Rotate::with_options(
            "log/%Y-%m-%d-size-test.log".to_string(),
            1024, // 1 KB
            7,    // 保留 7 天
        );

        let now = Local::now();

        // 寫入超過 1KB 的資料，觸發輪轉
        for i in 0..20 {
            let msg = format!(
                "{} Line {} - {}\r\n",
                now.format("%F %X%.6f"),
                i,
                "X".repeat(100)
            );
            if let Err(why) = r.write_msg(now, msg.as_bytes()) {
                logging::error_console(format!("Failed to write line {}: {:?}", i, why));
            }
        }

        logging::debug_file_async(format!(
            "最終 generation: {}, current_size: {}",
            r.generation, r.current_size
        ));
        logging::debug_file_async("結束 test_size_rotation".to_string());
    }

    /// 驗證外部設定的套用、範圍收斂，以及 `Rotate::new` 會跟隨全域設定。
    ///
    /// 測試結束時還原預設值，避免影響同一 process 內的其他測試。
    #[test]
    fn test_configure_limits() {
        // 0 代表未設定，維持既有值
        configure(0, 0);
        assert_eq!(
            effective_settings(),
            (DEFAULT_MAX_SIZE, DEFAULT_MAX_AGE_DAYS)
        );

        // 一般設定值直接生效
        configure(20, 14);
        assert_eq!(effective_settings(), (20 * 1024 * 1024, 14));

        // 已建立的 Rotate（跟隨全域）也會讀到新值
        let follower = Rotate::new("log/%Y-%m-%d-configure-test.log".to_string());
        assert_eq!(follower.max_size(), 20 * 1024 * 1024);
        assert_eq!(follower.max_age(), TimeDelta::days(14));

        // 以固定值建立者不受全域影響
        let fixed = Rotate::with_options("log/%Y-%m-%d-configure-test.log".to_string(), 4096, 3);
        assert_eq!(fixed.max_size(), 4096);
        assert_eq!(fixed.max_age(), TimeDelta::days(3));

        // 超出範圍者收斂至邊界
        configure(4096, 3650);
        assert_eq!(
            effective_settings(),
            (MAX_CONFIGURABLE_SIZE, MAX_CONFIGURABLE_AGE_DAYS)
        );

        // 還原預設值
        GLOBAL_MAX_SIZE.store(DEFAULT_MAX_SIZE, Ordering::Relaxed);
        GLOBAL_MAX_AGE_DAYS.store(DEFAULT_MAX_AGE_DAYS, Ordering::Relaxed);
    }

    /// 驗證輪轉檔名以「開檔時間」標記，並在同秒撞名時遞增序號。
    #[test]
    fn test_timestamp_filename() {
        let r = Rotate::new("log/%Y-%m-%d-app.log".to_string());
        let base = "log/2025-02-03-app.log";
        let sep = std::path::MAIN_SEPARATOR;

        assert_eq!(
            r.generate_full_fn(base, "00-05-32", 0),
            format!("log{sep}2025-02-03-app.00-05-32.log")
        );
        assert_eq!(
            r.generate_full_fn(base, "01-03-07", 0),
            format!("log{sep}2025-02-03-app.01-03-07.log")
        );
        assert_eq!(
            r.generate_full_fn(base, "01-03-07", 2),
            format!("log{sep}2025-02-03-app.01-03-07-2.log")
        );
    }

    /// 驗證輪轉後的檔案不會互相覆蓋，且檔名都帶時間戳
    #[tokio::test]
    #[ignore]
    async fn test_no_overwrite() {
        use std::collections::HashSet;

        dotenvy::dotenv().ok();

        // 清理舊測試檔案
        if let Ok(entries) = fs::read_dir("log") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("no-overwrite-test") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }

        // 設定很小的檔案大小限制 (512 bytes) 來強制多次輪轉
        let mut r = Rotate::with_options(
            "log/%Y-%m-%d-no-overwrite-test.log".to_string(),
            512, // 512 bytes
            7,
        );

        let now = Local::now();

        // 寫入足夠多的資料來觸發多次輪轉
        for i in 0..50 {
            let msg = format!("Line {:03} - {}\r\n", i, "X".repeat(50));
            r.write_msg(now, msg.as_bytes()).unwrap();
        }

        let final_generation = r.generation;
        println!("最終 generation: {}", final_generation);

        // 驗證產生了多個檔案
        assert!(
            final_generation >= 3,
            "應該至少輪轉 3 次，實際: {}",
            final_generation
        );

        // 收集所有產生的檔案
        let mut files: HashSet<String> = HashSet::new();
        if let Ok(entries) = fs::read_dir("log") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.contains("no-overwrite-test") {
                    files.insert(name);
                }
            }
        }

        // 驗證檔案數量 = 輪轉次數 + 1（第一個檔案不算輪轉）。
        let expected_files = final_generation + 1;
        assert_eq!(
            files.len() as u32,
            expected_files,
            "檔案數量應為 {}，實際: {}。檔案: {:?}",
            expected_files,
            files.len(),
            files
        );

        // 本測試所有寫入共用同一個 now，因此時間戳相同、以序號區分。
        let base_fn = now.format("%Y-%m-%d-no-overwrite-test").to_string();
        let stamp = now.format("%H-%M-%S").to_string();
        for seq in 0..=final_generation {
            let expected_name = if seq == 0 {
                format!("{}.{}.log", base_fn, stamp)
            } else {
                format!("{}.{}-{}.log", base_fn, stamp, seq)
            };
            assert!(
                files.contains(&expected_name),
                "缺少檔案: {}。實際檔案: {:?}",
                expected_name,
                files
            );
        }

        println!("驗證通過: 產生了 {} 個檔案，無覆蓋", files.len());
    }
}
