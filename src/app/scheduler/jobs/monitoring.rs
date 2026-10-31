//! 固定間隔的主機與資料庫健康檢查。
use crate::app::event;

use super::JobDefinition;

/// 本時段的排程，依執行時間排序。
pub(super) fn definitions() -> Vec<JobDefinition> {
    vec![
        // 每 10 分鐘檢查 pi 的供電／降頻、可用記憶體與 swap、SD 卡寫入耗時，異常時告警（同類 6 小時一次）；
        // 一天 144 輪，task.begin／task.done 記 DEBUG。非 pi 讀不到的項目略過。
        job!(
            quiet,
            "0 */10 * * * *",
            "主機健康檢查",
            event::host_health::execute,
        ),
        // 每 5 分鐘（第 30 秒，避開整分的排程）用獨立短連線探測正式庫；連續 2 次連不上、恢復、
        // 重啟（附統計是否重置）時告警。一天 288 輪，task.begin／task.done 記 DEBUG。
        job!(
            quiet,
            "30 */5 * * * *",
            "資料庫健康檢查",
            event::db_health::execute,
        ),
    ]
}
