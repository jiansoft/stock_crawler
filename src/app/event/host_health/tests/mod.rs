//! 主機健康檢查的測試：共用樣本在這裡，各主題的測試在子模組。

use super::*;

/// 測試用的時間點。
fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, day)
        .unwrap()
        .and_hms_opt(hour, minute, 0)
        .unwrap()
}

/// 一切正常的樣本：可用 700 MB、swap 用 50%、fsync 12 ms、開機以來沒有供電異常、不在部署期間。
fn healthy() -> Sample {
    Sample {
        throttled: Some(0),
        since_boot: Some(0),
        memory: Some(MemInfo {
            available_kib: 700 * 1024,
            swap_total_kib: 200 * 1024,
            swap_free_kib: 100 * 1024,
        }),
        probe: Probe::Done(Duration::from_millis(12)),
        deploy: None,
    }
}

/// 讀不到任何資料的樣本。
fn empty(probe: Probe) -> Sample {
    Sample {
        throttled: None,
        since_boot: None,
        memory: None,
        probe,
        deploy: None,
    }
}

/// 部署標記：`minutes` 分鐘前最後異動，`finished` 表示腳本已寫入結束。
fn marker(minutes: u64, finished: bool) -> Option<DeployMarker> {
    Some(DeployMarker {
        unit: "stock_go-web".to_owned(),
        age: Duration::from_secs(minutes * 60),
        finished,
    })
}

/// 部署時常見的樣本：swap 用滿、fsync 22.6 秒。
fn deploying(deploy: Option<DeployMarker>) -> Sample {
    Sample {
        memory: Some(MemInfo {
            available_kib: 300 * 1024,
            swap_total_kib: 200 * 1024,
            swap_free_kib: 0,
        }),
        probe: Probe::Done(Duration::from_millis(22_600)),
        deploy,
        ..healthy()
    }
}

/// 測試專用的暫存目錄（每個測試不同名稱，結束時刪除）。
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "stock_crawler_host_health_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

mod evaluate;
mod gate;
mod io;
mod parse;
