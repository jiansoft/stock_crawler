//! 舊日誌清理：刪除同目錄下超過保留天數的檔案（只處理目前檔名模式產生的檔案）。

use std::{
    fs::{self},
    io::{self},
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::UNIX_EPOCH,
};

use anyhow::Result;
use chrono::{DateTime, Local};
use rayon::prelude::*;

use crate::core::logging;

use super::Rotate;

impl Rotate {
    /// 清理舊檔案（超過 max_age 的檔案）
    pub(super) fn cleanup_old_files(&self, now: DateTime<Local>) {
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
    pub(super) fn files_in_directory<P: AsRef<Path>>(
        file_path: P,
    ) -> Result<Vec<PathBuf>, io::Error> {
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
