//! 部署標記（`/dev/shm/stock-deploying/<unit>`）：解析標記內容、找出最近異動的標記。

use super::*;

/// 最近一次異動的部署標記。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeployMarker {
    /// 部署單元（標記內容的 `unit=`，沒有就用檔名）。
    pub(crate) unit: String,
    /// 距標記最後異動多久。
    pub(crate) age: Duration,
    /// 部署腳本是否已寫入結束。
    pub(crate) finished: bool,
}

impl DeployMarker {
    /// 是否仍在部署期間（含結束後的緩衝）。
    pub(super) fn active(&self) -> bool {
        self.age <= DEPLOY_GRACE
    }

    /// 沒寫入結束又超過時間：部署腳本中途被砍掉留下的殘留標記。
    pub(super) fn stale(&self) -> bool {
        !self.finished && !self.active()
    }
}

/// 解析部署標記內容；`unit=` 缺少時用 `fallback_unit`（檔名）。
pub(crate) fn parse_deploy_marker(text: &str, fallback_unit: &str, age: Duration) -> DeployMarker {
    let unit = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("unit="))
        .filter(|unit| !unit.is_empty())
        .unwrap_or(fallback_unit)
        .to_owned();
    DeployMarker {
        unit,
        age,
        finished: text
            .lines()
            .any(|line| line.trim().starts_with("finished=")),
    }
}

/// 讀取部署標記目錄，回傳最近異動的一個；目錄不存在或沒有標記回 `None`。
pub(crate) fn read_deploy_marker(dir: &Path, now: SystemTime) -> Option<DeployMarker> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let age = now
                .duration_since(metadata.modified().ok()?)
                .unwrap_or_default();
            let text = read_small(&entry.path()).unwrap_or_default();
            Some(parse_deploy_marker(
                &text,
                &entry.file_name().to_string_lossy(),
                age,
            ))
        })
        .min_by_key(|marker| marker.age)
}
