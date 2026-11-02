use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::Path,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::Result;
use chrono::{DateTime, Local, TimeDelta};

use crate::core::logging;

mod cleanup;
mod settings;

use settings::{DEFAULT_MAX_AGE_DAYS, GLOBAL_MAX_AGE_DAYS, GLOBAL_MAX_SIZE, MAX_FILENAME_SEQ};
pub use settings::{configure, effective_settings};

/// 依日期與大小自動輪轉的檔案 writer。
///
/// 命名策略：**正在寫入的檔案永遠不帶時間戳**（例如
/// `log/2026-08-14_default_info.log`）；當檔案超過大小上限時，先把它更名為
/// `log/2026-08-14_default_info.09-06-56.log`（時間戳為該檔最後一筆日誌的時間），
/// 再開一個同名的空檔繼續寫。因此最新日誌一律位於固定檔名上，方便 tail 與監控。
pub struct Rotate {
    /// 檔名模式，例如 "log/%Y-%m-%d-name.log"
    fn_pattern: String,
    /// 當前完整檔名（即不帶時間戳的活躍檔名）
    cur_fn: String,
    /// 當前完整檔名的同步保護鎖。
    cur_fn_lock: RwLock<String>,
    /// 當前基礎檔名（不含時間戳，由日期決定；同時就是活躍檔名，並用於偵測跨日）
    cur_base_fn: String,
    /// 檔案輸出 handle
    out_fh: Option<Arc<RwLock<BufWriter<File>>>>,
    /// 當日已輪轉次數，僅供診斷與測試觀察（封存檔名以時間戳區分）
    generation: u32,
    /// 單檔最大大小 (bytes)；`None` 表示跟隨全域設定
    max_size: Option<u64>,
    /// 當前檔案已寫入大小
    current_size: u64,
    /// 當前檔案最後一筆日誌的時間；輪轉時作為封存檔名的時間戳
    last_write_at: Option<DateTime<Local>>,
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
            last_write_at: None,
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
    ///
    /// 跨日時舊日期的活躍檔（檔名已含日期，不會與新檔撞名）保持原樣，
    /// 直接改開新日期的活躍檔。
    fn ensure_writer(&mut self, now: DateTime<Local>) -> Result<()> {
        let base_fn = self.generate_base_fn(now);

        // 日期未變更且檔案已開啟：沿用現有 writer
        if base_fn == self.cur_base_fn && self.out_fh.is_some() {
            return Ok(());
        }

        // 跨日（或首次開檔）：重設輪轉計數並開新檔
        self.generation = 0;
        self.current_size = 0;
        self.last_write_at = None;
        self.cur_base_fn = base_fn;
        self.open_current_file()?;
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
        drop(w);

        self.current_size += msg.len() as u64;
        self.last_write_at = Some(now);

        Ok(())
    }

    /// 產生基礎檔名（根據日期，不含時間戳）
    fn generate_base_fn(&self, now: DateTime<Local>) -> String {
        now.format(&self.fn_pattern).to_string()
    }

    /// 產生封存檔名（含時間戳）
    ///
    /// 時間戳為該檔**最後一筆**日誌的時間（時-分-秒），排查時可直接由檔名定位時段。
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

    /// 依封存時間挑出尚未被使用的封存檔名。
    ///
    /// 同一秒內多次輪轉（或重啟）時遞增序號，確保不會覆蓋既有封存檔。
    fn resolve_archive_fn(&self, stamp_at: DateTime<Local>) -> String {
        let stamp = stamp_at.format("%H-%M-%S").to_string();
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

    /// 開啟（或接續）當前活躍檔案，檔名固定為不含時間戳的 `cur_base_fn`。
    ///
    /// 以 append 模式開啟：程序重啟後會接續寫入同一天既有的活躍檔，
    /// 檔案大小由 metadata 取得，重啟後仍能正確判斷是否該輪轉。
    fn open_current_file(&mut self) -> Result<()> {
        // 先 flush 並關閉舊檔案
        self.close_current();

        // 確保目錄存在
        if let Some(parent) = Path::new(&self.cur_base_fn).parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }

        let filename = self.cur_base_fn.clone();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&filename)?;

        // 取得現有檔案大小（重啟後接續既有檔）
        self.current_size = file.metadata().map(|m| m.len()).unwrap_or(0);

        self.out_fh = Some(Arc::new(RwLock::new(BufWriter::with_capacity(4096, file))));
        self.cur_fn = filename;

        if let Ok(mut lock) = self.cur_fn_lock.write() {
            lock.clone_from(&self.cur_fn);
        }

        Ok(())
    }

    /// 執行輪轉（因大小超限）。
    ///
    /// 先把活躍檔更名為帶時間戳的封存檔（時間戳取該檔最後一筆日誌的時間，
    /// 尚無寫入紀錄時退回當下時間），再開一個同名的空活躍檔繼續寫。
    /// 更名前必須先關閉檔案 handle，否則 Windows 會拒絕更名。
    fn rotate_by_size(&mut self, now: DateTime<Local>) -> Result<()> {
        self.close_current();

        let stamp_at = self.last_write_at.unwrap_or(now);
        let archive_fn = self.resolve_archive_fn(stamp_at);

        if let Err(why) = fs::rename(&self.cur_base_fn, &archive_fn) {
            // 更名失敗（例如檔案被其他程序鎖住）時不中斷日誌流程，
            // 續寫原檔並在下一筆訊息再嘗試輪轉。
            logging::error_console(format!(
                "couldn't rotate the log file({} -> {}). because {:?}",
                self.cur_base_fn, archive_fn, why
            ));
        } else {
            self.generation += 1;
        }

        self.last_write_at = None;

        // current_size 由 open_current_file 依實際檔案大小重設
        self.open_current_file()
    }

    /// flush 當前檔案
    fn flush_current(&self) {
        if let Some(ref writer) = self.out_fh
            && let Ok(mut w) = writer.write()
        {
            let _ = w.flush();
        }
    }

    /// flush 並關閉當前檔案 handle，讓檔案可被更名。
    fn close_current(&mut self) {
        self.flush_current();
        self.out_fh = None;
    }
}

impl Drop for Rotate {
    fn drop(&mut self) {
        self.flush_current();
    }
}

#[cfg(test)]
mod tests;
