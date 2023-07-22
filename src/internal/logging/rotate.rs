use core::result::Result::Ok;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
    time::UNIX_EPOCH,
};

use anyhow::*;
use chrono::{DateTime, Local};
use rayon::prelude::*;

use crate::internal::logging;

pub struct Rotate {
    /// log/%Y-%m-%d-name.log
    fn_pattern: String,
    cur_fn: String,
    cur_fn_lock: RwLock<String>,
    cur_base_fn: String,
    out_fh: Option<Arc<RwLock<BufWriter<File>>>>,
    generation: i64,
    max_age: chrono::Duration,
    on_rotate: AtomicBool,
}

impl Rotate {
    pub fn new(fn_pattern: String) -> Self {
        Rotate {
            fn_pattern,
            generation: 0,
            cur_fn: "".to_string(),
            cur_fn_lock: Default::default(),
            cur_base_fn: "".to_string(),
            out_fh: None,
            max_age: chrono::Duration::days(7),
            on_rotate: Default::default(),
        }
    }

    pub fn get_writer(&mut self, now: DateTime<Local>) -> Option<Arc<RwLock<BufWriter<File>>>> {
        let base_fn = self.generate_fn(now);
        if base_fn == self.cur_base_fn {
            return self.out_fh.clone();
        }

        let filename = base_fn.clone();
        let mut generation = self.generation;
        match self.cur_fn_lock.write() {
            Ok(mut cur_fn) => {
                let file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .truncate(false)
                    .open(&filename)
                    .unwrap_or_else(|e| {
                        panic!("Failed to open log file: {}", e);
                    });

                generation = 0;

                self.out_fh = Some(Arc::new(RwLock::new(BufWriter::with_capacity(2048, file))));
                self.cur_base_fn = base_fn;
                self.cur_fn = filename.to_string();
                self.generation = generation;
                self.rotate(now);

                *cur_fn = filename;
            }
            Err(why) => {
                logging::error_console(format!("Failed to cur_fn_lock.write because:{:?}", why));
                return None;
            }
        }

        self.out_fh.clone()
    }

    /// 產生檔案名稱
    fn generate_fn(&self, now: DateTime<Local>) -> String {
        now.format(&self.fn_pattern).to_string()
    }

    fn rotate(&self, now: DateTime<Local>) {
        if self.on_rotate.swap(true, Ordering::Relaxed) {
            return;
        }

        //self.on_rotate.store(true, Ordering::Relaxed);

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
                                    &unlink.display().to_string(),
                                    why
                                ));
                            }
                            Ok(_) => {
                                logging::info_file_async(format!(
                                    "the file has been deleted:{}",
                                    &unlink.display().to_string()
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
        let parent_dir = path.parent().ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Parent directory not found",
        ))?;

        let mut files = Vec::new();
        for entry in fs::read_dir(parent_dir)? {
            let entry = entry?;
            let file_path = entry.path();
            files.push(file_path);
        }

        Ok(files)
    }

    /* fn list_files_in_directory<P: AsRef<Path>>(file_path: P) -> Result<Vec<String>> {
        let path = file_path.as_ref();
        let parent_dir = path.parent().ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Parent directory not found",
        ))?;

        let mut files = Vec::new();
        for entry in fs::read_dir(parent_dir)? {
            let entry = entry?;
            let file_path = entry.path().display().to_string();
            files.push(file_path);
        }
        Ok(files)
    }*/
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use crate::internal::logging;

    use super::*;

    #[tokio::test]
    async fn test_init() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 execute".to_string());
        let mut now = Local::now();
        logging::debug_file_async("第一次test_write_all".to_string());
        test_write_all(now);
        now += chrono::Duration::days(1);
        logging::debug_file_async("第二次test_write_all".to_string());
        test_write_all(now);

        logging::debug_file_async("結束 execute".to_string());
    }

    fn test_write_all(now: DateTime<Local>) {
        let mut r = Rotate::new("log/%Y-%m-%d-test.log".to_string());
        if let Some(writer) = r.get_writer(now) {
            match writer.write() {
                Ok(mut w) => {
                    let line = format!("{} 測試\r\n", now.format("%F %X%.6f"));
                    if let Err(why) = w.write_all(line.as_bytes()) {
                        logging::error_console(format!(
                            "Failed to write to log file. because:{:#?}\r\nmsg:{}",
                            why, line
                        ));
                    }
                }
                Err(why) => {
                    logging::debug_file_async(format!("Failed to writer.write because {:?}", why));
                }
            }
        }
    }

    #[tokio::test]
    async fn test_rotate() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 test_rotate".to_string());
        let mut r = Rotate::new("log/%Y-%m-%d-test.log".to_string());
        if r.get_writer(Local::now()).is_some() {}

        logging::debug_file_async("結束 test_rotate".to_string());
    }
}
