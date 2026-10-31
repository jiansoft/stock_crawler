//! 08:00～14:59 開盤前後：除權息與股利提醒、權值佔比、股價高低標監控。
use crate::app::backfill::stock_weight;
use crate::app::event;

use super::JobDefinition;

/// 本時段的排程，依執行時間排序。
pub(super) fn definitions() -> Vec<JobDefinition> {
    vec![
        // 08:00 提醒本日除權息與明日預計除權息的股票
        job!(
            normal,
            "0 0 8 * * *",
            "提醒本日與明日除權息股票",
            event::taiwan_stock::ex_dividend::execute,
        ),
        // 08:02 提醒本日發放股利的股票(只通知自已有的股票)
        job!(
            normal,
            "0 2 8 * * *",
            "提醒本日持股股利發放",
            event::taiwan_stock::payable_date::execute,
        ),
        // 08:04 提醒本日開始公開申購的股票
        job!(
            normal,
            "0 4 8 * * *",
            "提醒本日公開申購股票",
            event::taiwan_stock::public::execute,
        ),
        // 09:00 更新股票權值佔比
        job!(
            normal,
            "0 0 9 * * *",
            "更新股票加權權值佔比",
            stock_weight::execute
        ),
        // 09:02 提醒本日已達高低標的股票有那些
        job!(
            normal,
            "0 2 9 * * *",
            "監控並提醒股價達高低標",
            event::trace::stock_price::execute,
        ),
    ]
}
