use crate::internal::logging;
use anyhow::*;
use chrono::{DateTime, Local};
use core::result::Result::Ok;
use rayon::prelude::*;
use std::{
    fs,
    fs::{File, OpenOptions},
    io,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::UNIX_EPOCH,
};

pub struct Rotate {
    /// log/%Y-%m-%d-name.log
    pub fn_pattern: String,
    pub cur_fn: String,
    pub cur_base_fn: String,
    pub out_fh: Option<Arc<RwLock<BufWriter<File>>>>,
    pub generation: i64,
    pub max_age: chrono::Duration,
}

impl Rotate {
    pub fn new(fn_pattern: String) -> Self {
        Rotate {
            fn_pattern,
            generation: 0,
            cur_fn: "".to_string(),
            cur_base_fn: "".to_string(),
            out_fh: None,
            max_age: chrono::Duration::days(7),
        }
    }

    pub fn get_writer(&mut self, now: DateTime<Local>) -> Option<Arc<RwLock<BufWriter<File>>>> {
        let mut generation = self.generation;

        let base_fn = self.generate_fn(now);
        let filename = base_fn.clone();

        if base_fn == self.cur_base_fn {
            let arc = self.out_fh.clone();
            return arc;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .truncate(false)
            .open(&filename)
            .unwrap_or_else(|e| {
                panic!("Failed to open log file: {}", e);
            });

        generation = 0;

        self.out_fh = Some(Arc::new(RwLock::new(BufWriter::new(file))));
        self.cur_base_fn = base_fn;
        self.cur_fn = filename;
        self.generation = generation;

        self.rotate(now);

        self.out_fh.clone()
    }

    /// 產生檔案名稱
    pub fn generate_fn(&self, now: DateTime<Local>) -> String {
        now.format(&self.fn_pattern).to_string()
    }

    pub fn rotate(&self, now: DateTime<Local>) {
        //logging::debug_file_async(format!("self.cur_fn:{}", &self.cur_fn));
        match Self::list_files_in_directory(&self.cur_fn) {
            Ok(files) => {
                let cut_off = (now - self.max_age).timestamp() as u64;
                //logging::debug_file_async(format!("cut_off:{}", cut_off));
                /*
                let mut to_unlink: Vec<PathBuf> = Vec::with_capacity(files.len());
                for file in files {
                    if let Ok(metadata) = fs::metadata(&file) {
                        if let Ok(system_time) = metadata.modified() {
                            if let Ok(file_duration) = system_time.duration_since(UNIX_EPOCH) {
                                if file_duration.as_secs() <= cut_off {
                                    to_unlink.push(file)
                                }
                            }
                        }
                    }
                }
                */

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

                if to_unlink.is_empty() {
                    return;
                }

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
            Err(why) => {
                logging::error_console(format!(
                    "Failed to list_files_in_directory because {:?}",
                    why
                ));
            }
        }
    }

    fn list_files_in_directory<P: AsRef<Path>>(file_path: P) -> Result<Vec<PathBuf>, io::Error> {
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
    use super::*;
    use crate::internal::logging;
    use std::io::Write;

    #[tokio::test]
    async fn test_init() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 execute".to_string());
        let mut now = Local::now();
        test_write_all(now);
        now += chrono::Duration::days(1);
        test_write_all(now);

        logging::debug_file_async("結束 execute".to_string());
    }

    fn test_write_all(now: DateTime<Local>) {
        let mut r = Rotate::new("log/%Y-%m-%d-test.log".to_string());
        let base_fn = r.generate_fn(now);
        println!("base_fn:{}", base_fn);
        r.rotate(now);

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
        if let Some(_) = r.get_writer(Local::now()) {
            r.rotate(Local::now());
        }

        logging::debug_file_async("結束 test_rotate".to_string());
    }
}
