//! 01:00～07:59 盤前批次：財報、營收、股利與基本資料的採集與重算。
use crate::app::backfill::{
    capital_reduction, delisted_company, dividend, etf, financial_report, financial_statement,
    isin, net_asset_value_per_share, revenue,
};
use crate::app::calculation;
use crate::app::event;

use super::JobDefinition;

/// 本時段的排程，依執行時間排序。
pub(super) fn definitions() -> Vec<JobDefinition> {
    vec![
        // 01:00 更新興櫃股票的每股淨值
        job!(
            normal,
            "0 0 1 * * *",
            "更新興櫃股票每股淨值",
            net_asset_value_per_share::emerging::execute,
        ),
        // 02:30 更新盈餘分配率
        job!(
            normal,
            "0 30 2 * * *",
            "更新盈餘分配率",
            dividend::payout_ratio::execute,
        ),
        // 03:00 更新台股季度財報
        job!(
            normal,
            "0 0 3 * * *",
            "更新台股季度財報",
            event::taiwan_stock::quarter_eps::execute,
        ),
        // 04:00 更新台股季度財報(ROE、ROA為零的數據)
        job!(
            normal,
            "0 0 4 * * *",
            "補齊季報零淨值之 ROE/ROA 數據",
            financial_statement::quarter::execute,
        ),
        // 05:00 更新台股年度財報(僅有eps 等少數欄位的資料)
        job!(
            normal,
            "0 0 5 * * *",
            "更新年度財報基本 EPS",
            event::taiwan_stock::annual_eps::execute,
        ),
        // 05:05 更新台股年度財報
        job!(
            normal,
            "0 5 5 * * *",
            "更新完整年度財報",
            financial_statement::annual::execute,
        ),
        // 05:10 從yahoo取得每股淨值數據，將未下市但每股淨值為零的股票更新其數據
        job!(
            normal,
            "0 10 5 * * *",
            "更新每股淨值(補 Yahoo 零淨值數據)",
            net_asset_value_per_share::zero_value::execute,
        ),
        // 05:15 取得台股的營收
        job!(normal, "0 15 5 * * *", "取得台股月度營收", revenue::execute),
        // 05:20 更新台股國際證券識別碼
        job!(
            normal,
            "0 20 5 * * *",
            "更新台股 ISIN 識別碼",
            isin::execute
        ),
        // 05:25 更新下市的股票
        job!(
            normal,
            "0 25 5 * * *",
            "更新下市股票清單",
            delisted_company::execute,
        ),
        // 05:30 更新台股 ETF 資訊
        job!(normal, "0 30 5 * * *", "更新台股 ETF 資訊", etf::execute),
        // 05:32 回補交易所公告的減資事件
        // 上櫃的 revivt 公告只涵蓋當週、無法查歷史，因此必須每天抓才不會漏。
        // 必須排在 05:40 的 CAGR 之前：減資造成的價格跳動要先登錄成公司行動，
        // 當日的報酬率才不會把它誤算成真實漲跌。
        job!(
            normal,
            "0 32 5 * * *",
            "回補交易所公告的減資事件",
            capital_reduction::execute,
        ),
        // 05:35 掃描交易所除權息公告，補齊漏抓的股利事件
        // 必須排在 05:40 的 CAGR 之前，讓當日新補進來的股利能立刻被納入計算
        job!(
            normal,
            "0 35 5 * * *",
            "掃描除權息公告補齊漏抓事件",
            dividend::scan_announcements,
        ),
        // 05:37 以交易所除權除息計算結果核對近 45 天的股利日期與金額
        // Yahoo 會留著延後除息前的舊日期；必須排在 05:40 的 CAGR 之前，含息報酬才會用到正確的除息日
        job!(
            normal,
            "0 37 5 * * *",
            "核對交易所除權息結果",
            dividend::reconcile_ex_right_results,
        ),
        // 05:40 計算各期間年化報酬率(CAGR)
        // 必須排在 21:00 年度配息回補與 05:00~05:30 各項回補之後，
        // 否則當日結果會少算前一晚才補進來的股利
        job!(
            normal,
            "0 40 5 * * *",
            "計算各期間年化報酬率(CAGR)",
            calculation::cagr::execute_scheduled,
        ),
        // 06:00 採集 Yahoo 三大財務報表（損益表、資產負債表、現金流量表）
        // 每輪最多 400 檔、約 45 分鐘；7 天 Redis 旗標讓每檔約每週重抓一次。
        // Yahoo 失敗（404 除外）的股票改從 BigGo 財經備援。
        // 避開 21:00 的 Yahoo 股利採集，兩者不會同時對 Yahoo 發請求。
        job!(
            normal,
            "0 0 6 * * *",
            "採集 Yahoo 三大財務報表",
            financial_report::execute,
        ),
        // 07:10 以三大財務報表重算全市場財務體質（F 分數、毛利率、負債比、近四季自由現金流），
        // 排在 06:00 採集（約 35～45 分鐘）之後；條件選股以這張表篩選。
        job!(
            normal,
            "0 10 7 * * *",
            "重算財務體質評分",
            calculation::financial_health::execute,
        ),
        // 07:30（每月 1–10 日）當月第一個交易日才跑：策略優勢滾動檢查，對要通知的策略跑近 24 個月與
        // 2019 起的換股日偏移掃描（次日開盤＋20 bps），輸 0050 或勝率不到一半就告警（不自動停用通知）；
        // 排在 07:10 財務體質重算之後、08:00 提醒之前。
        job!(
            normal,
            "0 30 7 1-10 * *",
            "策略優勢滾動檢查",
            event::taiwan_stock::strategy_edge::execute,
        ),
    ]
}
