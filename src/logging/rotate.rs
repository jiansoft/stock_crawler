use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::UNIX_EPOCH,
};

use anyhow::Result;
use chrono::{DateTime, Local, TimeDelta};
use rayon::prelude::*;

use crate::logging;

/// 預設單檔最大大小：10 MB
const DEFAULT_MAX_SIZE: u64 = 10 * 1024 * 1024;
/// 預設保留天數：7 天
const DEFAULT_MAX_AGE_DAYS: i64 = 7;

pub struct Rotate {
    /// 檔名模式，例如 "log/%Y-%m-%d-name.log"
    fn_pattern: String,
    /// 當前完整檔名（含 generation）
    cur_fn: String,
    cur_fn_lock: RwLock<String>,
    /// 當前基礎檔名（不含 generation，由日期決定）
    cur_base_fn: String,
    /// 檔案輸出 handle
    out_fh: Option<Arc<RwLock<BufWriter<File>>>>,
    /// 當前世代編號 (0, 1, 2, ...)，只增不減
    generation: u32,
    /// 單檔最大大小 (bytes)
    max_size: u64,
    /// 當前檔案已寫入大小
    current_size: u64,
    /// 日誌保留時間
    max_age: chrono::Duration,
    /// 是否正在執行輪轉
    on_rotate: AtomicBool,
}

impl Rotate {
    /// 使用預設設定建立 Rotate 實例
    ///
    /// 預設值：
    /// - max_size: 10 MB
    /// - max_age: 7 天
    pub fn new(fn_pattern: String) -> Self {
        Self::with_options(fn_pattern, DEFAULT_MAX_SIZE, DEFAULT_MAX_AGE_DAYS)
    }

    /// 使用自訂設定建立 Rotate 實例
    ///
    /// # Arguments
    /// * `fn_pattern` - 檔名模式，例如 "log/%Y-%m-%d-app.log"
    /// * `max_size` - 單檔最大大小 (bytes)
    /// * `max_age_days` - 日誌保留天數
    pub fn with_options(fn_pattern: String, max_size: u64, max_age_days: i64) -> Self {
        Rotate {
            fn_pattern,
            cur_fn: String::new(),
            cur_fn_lock: Default::default(),
            cur_base_fn: String::new(),
            out_fh: None,
            generation: 0,
            max_size,
            current_size: 0,
            max_age: TimeDelta::try_days(max_age_days).unwrap_or(TimeDelta::days(7)),
            on_rotate: Default::default(),
        }
    }

    /// 取得檔案寫入器
    pub fn get_writer(&mut self, now: DateTime<Local>) -> Option<Arc<RwLock<BufWriter<File>>>> {
        let base_fn = self.generate_base_fn(now);

        // 日期變更：重設 generation
        if base_fn != self.cur_base_fn {
            self.generation = 0;
            self.current_size = 0;
            self.cur_base_fn = base_fn;

            if let Err(why) = self.open_new_file() {
                logging::error_console(format!("Failed to open new log file: {:?}", why));
                return None;
            }

            self.cleanup_old_files(now);
        }

        self.out_fh.clone()
    }

    /// 寫入日誌訊息，自動處理大小檢查和世代輪轉
    ///
    /// # Arguments
    /// * `now` - 當前時間
    /// * `msg` - 要寫入的訊息
    pub fn write_msg(&mut self, now: DateTime<Local>, msg: &[u8]) -> Result<()> {
        // 確保有有效的 writer
        if self.get_writer(now).is_none() {
            return Err(anyhow::anyhow!("Failed to get writer"));
        }

        // 檢查是否需要因大小超限而輪轉
        if self.should_rotate_by_size(msg.len()) {
            self.rotate_generation()?;
        }

        // 寫入訊息
        if let Some(ref writer) = self.out_fh {
            if let Ok(mut w) = writer.write() {
                w.write_all(msg)?;
                self.current_size += msg.len() as u64;
            }
        }

        Ok(())
    }

    /// 產生基礎檔名（根據日期，不含 generation）
    fn generate_base_fn(&self, now: DateTime<Local>) -> String {
        now.format(&self.fn_pattern).to_string()
    }

    /// 產生完整檔名（含 generation）
    ///
    /// generation = 0: "log/2025-02-03-app.log"
    /// generation = 1: "log/2025-02-03-app.1.log"
    /// generation = 2: "log/2025-02-03-app.2.log"
    fn generate_full_fn(&self, base_fn: &str, generation: u32) -> String {
        if generation == 0 {
            base_fn.to_string()
        } else {
            let path = Path::new(base_fn);
            let parent = path.parent().unwrap_or(Path::new(""));
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("log");
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("log");

            parent
                .join(format!("{}.{}.{}", stem, generation, ext))
                .to_string_lossy()
                .to_string()
        }
    }

    /// 檢查是否需要因大小超限而輪轉
    fn should_rotate_by_size(&self, additional_bytes: usize) -> bool {
        self.current_size + additional_bytes as u64 > self.max_size
    }

    /// 開啟新檔案
    fn open_new_file(&mut self) -> Result<()> {
        // 先 flush 並關閉舊檔案
        self.flush_current();

        let filename = self.generate_full_fn(&self.cur_base_fn, self.generation);

        // 確保目錄存在
        if let Some(parent) = Path::new(&filename).parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&filename)?;

        // 取得現有檔案大小
        self.current_size = file.metadata().map(|m| m.len()).unwrap_or(0);

        self.out_fh = Some(Arc::new(RwLock::new(BufWriter::with_capacity(4096, file))));
        self.cur_fn = filename;

        if let Ok(mut lock) = self.cur_fn_lock.write() {
            lock.clone_from(&self.cur_fn);
        }

        Ok(())
    }

    /// 執行世代輪轉（因大小超限）
    fn rotate_generation(&mut self) -> Result<()> {
        // flush 當前檔案
        self.flush_current();

        // 遞增世代（只增不減，不覆蓋舊檔案）
        self.generation += 1;
        self.current_size = 0;

        // 開啟新檔案
        self.open_new_file()
    }

    /// flush 當前檔案
    fn flush_current(&self) {
        if let Some(ref writer) = self.out_fh {
            if let Ok(mut w) = writer.write() {
                let _ = w.flush();
            }
        }
    }

    /// 清理舊檔案（超過 max_age 的檔案）
    fn cleanup_old_files(&self, now: DateTime<Local>) {
        if self.on_rotate.swap(true, Ordering::Relaxed) {
            return;
        }

        match Self::files_in_directory(&self.cur_fn) {
            Ok(files) => {
                let cut_off = (now - self.max_age).timestamp() as u64;
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

    use crate::logging;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_basic_write() {
        dotenv::dotenv().ok();
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

    #[tokio::test]
    #[ignore]
    async fn test_date_rotation() {
        dotenv::dotenv().ok();
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

    #[tokio::test]
    #[ignore]
    async fn test_size_rotation() {
        dotenv::dotenv().ok();
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

    #[tokio::test]
    #[ignore]
    async fn test_generation_filename() {
        let r = Rotate::new("log/%Y-%m-%d-app.log".to_string());

        let base = "log/2025-02-03-app.log";
        assert_eq!(r.generate_full_fn(base, 0), "log/2025-02-03-app.log");
        assert_eq!(r.generate_full_fn(base, 1), "log/2025-02-03-app.1.log");
        assert_eq!(r.generate_full_fn(base, 2), "log/2025-02-03-app.2.log");

        println!("All filename generation tests passed!");
    }

    /// 驗證 generation 只增不減，且不會覆蓋舊檔案
    #[tokio::test]
    #[ignore]
    async fn test_no_overwrite() {
        use std::collections::HashSet;

        dotenv::dotenv().ok();

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

        // 驗證檔案數量 = generation + 1 (因為 gen 從 0 開始)
        let expected_files = final_generation + 1;
        assert_eq!(
            files.len() as u32,
            expected_files,
            "檔案數量應為 {}，實際: {}。檔案: {:?}",
            expected_files,
            files.len(),
            files
        );

        // 驗證每個 generation 的檔案都存在
        let base_fn = now.format("%Y-%m-%d-no-overwrite-test").to_string();
        for gen in 0..=final_generation {
            let expected_name = if gen == 0 {
                format!("{}.log", base_fn)
            } else {
                format!("{}.{}.log", base_fn, gen)
            };
            assert!(
                files.contains(&expected_name),
                "缺少檔案: {}",
                expected_name
            );
        }

        println!("驗證通過: 產生了 {} 個檔案，無覆蓋", files.len());
    }
}
