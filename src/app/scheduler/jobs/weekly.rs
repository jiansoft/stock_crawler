//! 週六的週報與每週一次的資料整理。
use crate::app::backfill::etf_zone;
use crate::app::event;

use super::JobDefinition;

/// 本時段的排程，依執行時間排序。
pub(super) fn definitions() -> Vec<JobDefinition> {
    vec![
        // 10:20（週六）每週市場摘要：產業 RS 中位數與一週變化、外資／投信近 5 日產業淨買賣、
        // 持股 RS 一週變化；週五的籌碼 21:50 入庫、報酬收盤後回填，週六早上資料已齊。
        job!(
            normal,
            "0 20 10 * * Sat",
            "每週市場摘要",
            event::taiwan_stock::weekly_market::execute,
        ),
        // 10:40（週六）集保股權分散入庫（holder_distribution 與全部持股分級；開放資料只有最新一週），
        // 成功後送持股大戶週報：從資料庫讀上週與 4 週前比較；同一資料週只通知一次，入庫失敗就不送。
        job!(
            normal,
            "0 40 10 * * Sat",
            "集保股權分散入庫與持股大戶週報",
            event::taiwan_stock::holder_distribution::execute,
        ),
        // 10:50（週六）持股估值週報：本益比、股價淨值比位於自己近 5 年 ≥ 第 90 或 ≤ 第 10 百分位的持股；
        // 沒有就不發。
        job!(
            normal,
            "0 50 10 * * Sat",
            "通知持股估值週報",
            event::taiwan_stock::valuation_alert::execute,
        ),
        // 11:00（週六）資料健康週報：近 7 天的交易日完整性、均線、漲跌幅、衍生資料新鮮度、
        // 月營收與股利佔位列；這類問題來源不會報錯，只能定期量。
        job!(
            normal,
            "0 0 11 * * Sat",
            "資料健康週報",
            event::taiwan_stock::data_health::execute,
        ),
        // 12:10（週六）Yahoo ETF 頁：基本資料、分類、前十大持股、行業比重與資產分佈（約 15 分鐘），
        // 避開每天 06:00 的 Yahoo 財報採集。
        job!(
            normal,
            "0 10 12 * * Sat",
            "更新 ETF 基本資料與持股",
            etf_zone::execute_weekly_profiles,
        ),
        // 13:05（週六）MoneyDJ 補近 30 天的 ETF 淨值缺日與空欄（MIS 只給當天，排程失敗那天不補就永遠缺），
        // 排在 12:10 Yahoo 基本資料（約 15 分鐘）之後，兩個來源不同時抓。
        job!(
            normal,
            "0 5 13 * * Sat",
            "補 ETF 淨值缺日",
            etf_zone::fill_recent_nav_gaps,
        ),
    ]
}
