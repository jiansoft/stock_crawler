//! 固定間隔的主機與資料庫健康檢查，以及共用快取的定期刷新。
use crate::app::{calculation, event};

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
        // 每 6 小時（00:05、06:05、12:05、18:05）重新查詢全市場填息基準並覆蓋快取（TTL 7 小時），
        // 股利預估、持股填息與除權息提醒都直接命中；00:05 是換日後快取鍵改變的第一次。
        job!(
            quiet,
            "0 5 */6 * * *",
            "更新全市場填息基準快取",
            calculation::dividend_fill_hints::refresh_market_peers,
        ),
    ]
}
