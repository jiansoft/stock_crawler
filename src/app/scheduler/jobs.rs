//! 台北時區的完整排程清單；名稱、cron 與建構函式放在同一個定義，避免註冊與測試脫節。
use super::{create_job, create_quiet_job};
use crate::{
    app::backfill::{
        capital_reduction, chip, delisted_company, dividend, etf, etf_zone, financial_report,
        financial_statement, isin, net_asset_value_per_share, official_ratio,
        qualified_foreign_institutional_investor, revenue, stock_weight,
    },
    app::calculation,
    app::event,
};
use anyhow::Result;
use tokio_cron_scheduler::Job;

/// 一個排程的固定中繼資料與不執行任務的建構函式。
pub(super) struct JobDefinition {
    pub(super) cron: &'static str,
    pub(super) name: &'static str,
    pub(super) create: fn() -> Result<Job>,
}

/// 將任務函式與固定中繼資料綁在一起，保留一般與靜默兩種記錄模式。
macro_rules! job {
    (normal, $cron:literal, $name:literal, $task:path $(,)?) => {
        JobDefinition {
            cron: $cron,
            name: $name,
            create: || create_job($cron, $name, $task),
        }
    };
    (quiet, $cron:literal, $name:literal, $task:path $(,)?) => {
        JobDefinition {
            cron: $cron,
            name: $name,
            create: || create_quiet_job($cron, $name, $task),
        }
    };
}

/// 註冊順序與拆檔前相同；建構不觸發任何採集或通知。
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
        // 15:00 取得收盤報價數據
        job!(
            normal,
            "0 0 15 * * *",
            "取得每日收盤行情與報價",
            event::taiwan_stock::closing::execute,
        ),
        // 15:30（週一至週五）大盤趨勢變化通知：加權指數當天站上或跌破 60／240 日均線、
        // 均線排列改變才發；15:00 收盤事件已寫入加權指數，最新一筆不是今天（休市）就不發。
        job!(
            normal,
            "0 30 15 * * Mon-Fri",
            "通知大盤趨勢變化",
            event::taiwan_stock::market_regime_alert::execute,
        ),
        // 17:00、19:00（週一至週五）檢查當日收盤資料：上市或上櫃有成交的檔數明顯偏少
        // （來源尚未公布、暫時失敗）就重跑收盤匯總並告警，避免缺口被「缺漏補齊」的零量列藏起來。
        // 行情完整後再檢查技術訊號與未來報酬，沒寫入就只重算缺的那一步並告警；大盤指數沒寫入也重抓。
        job!(
            normal,
            "0 0 17,19 * * Mon-Fri",
            "檢查收盤資料完整性",
            event::taiwan_stock::closing::ensure_complete,
        ),
        // 17:20（週一至週五）通知今天填息的持股：17:00 已確認收盤資料完整（不完整會重跑），
        // 以「填息日是今天」判斷，同一次除權息只會通知一次；休市日沒有今天的收盤，不會發送。
        job!(
            normal,
            "0 20 17 * * Mon-Fri",
            "通知持股填息",
            event::taiwan_stock::dividend_fill_alert::execute,
        ),
        // 17:30（週一至週五）ETF 專區：證交所 MIS 預估淨值（投信約 17:00 前更新完）→ 補推斷分類 →
        // 含息績效 → 重建清單頁快照；淨值抓取失敗時其餘步驟照做並告警。
        job!(
            normal,
            "0 30 17 * * Mon-Fri",
            "更新 ETF 淨值與專區快照",
            etf_zone::execute_daily,
        ),
        // 17:45（週一至週五）持有 ETF 剛轉為高溢價、剛好連續折價滿 5 天，或單日申贖達 ±3% 就通知，排在 17:30 淨值入庫之後。
        job!(
            normal,
            "0 45 17 * * Mon-Fri",
            "通知持有 ETF 折溢價與申贖",
            event::taiwan_stock::etf_premium_alert::execute,
        ),
        // 20:30 通知持股近期法說會的摘要、展望與 Q&A 重點（BigGo 整理）
        // 法說會多在下午舉行，BigGo 通常當天或隔天產生摘要；摘要未產生的場次之後的排程再看。
        job!(
            normal,
            "0 30 20 * * *",
            "通知持股法說會摘要",
            event::taiwan_stock::earnings_call::execute,
        ),
        // 20:40（週一至週五）通知持股主力進出：主力買賣超佔成交量達門檻的股票與前幾名分點
        // 富邦證券（MoneyDJ）的分點資料於盤後更新；頁面日期不是今天的股票略過，休市日不會發訊息。
        job!(
            normal,
            "0 40 20 * * Mon-Fri",
            "通知持股主力進出",
            event::taiwan_stock::broker_flow::execute,
        ),
        // 20:45（每月 10–28 日）通知持股董監質押變動
        // 董監事持股明細約每月 18 日出表；與 Redis 保存的前一期比較，同月份只通知一次。
        job!(
            normal,
            "0 45 20 10-28 * *",
            "通知持股董監質押變動",
            event::taiwan_stock::insider_pledge::execute,
        ),
        // 10:20（週六）每週市場摘要：產業 RS 中位數與一週變化、外資／投信近 5 日產業淨買賣、
        // 持股 RS 一週變化；週五的籌碼 21:50 入庫、報酬收盤後回填，週六早上資料已齊。
        job!(
            normal,
            "0 20 10 * * Sat",
            "每週市場摘要",
            event::taiwan_stock::weekly_market::execute,
        ),
        // 10:30（週六）持股千張大戶比例週報：集保股權分散表每週六公布上週五的資料，
        // 與 Redis 中上週的快照比較；同一份資料只通知一次。
        job!(
            normal,
            "0 30 10 * * Sat",
            "通知持股千張大戶週報",
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
        // 11:00（週六）資料健康週報：近 7 天的交易日完整性、均線、漲跌幅、衍生資料新鮮度、
        // 月營收與股利佔位列；這類問題來源不會報錯，只能定期量。
        job!(
            normal,
            "0 0 11 * * Sat",
            "資料健康週報",
            event::taiwan_stock::data_health::execute,
        ),
        // 19:15（週一至週五）通知持股的重要技術訊號與相對強度轉折；排在 19:00 收盤完整性檢查之後，
        // 若有重跑收盤匯總，訊號也已重新計算。
        job!(
            normal,
            "0 15 19 * * Mon-Fri",
            "通知持股技術訊號",
            event::taiwan_stock::technical_alert::execute,
        ),
        // 21:00 資料庫內尚未有年度配息數據的股票取出後向第三方查詢後更新回資料庫
        job!(
            normal,
            "0 0 21 * * *",
            "補齊缺失之年度配息數據",
            dividend::execute
        ),
        // 21:40（週一至週五）通知持股三大法人與融資融券的明顯動作
        // 融資融券約 21:00 後公布；外資、投信連續買賣超天數存在 Redis，同一天重跑不重複累加。
        job!(
            normal,
            "0 40 21 * * Mon-Fri",
            "通知持股法人與信用交易",
            event::taiwan_stock::chip_flow::execute,
        ),
        // 21:50（週一至週五）全市場三大法人買賣超與融資融券餘額入庫（chip_daily）
        job!(
            normal,
            "0 50 21 * * Mon-Fri",
            "籌碼資料入庫",
            chip::execute_daily
        ),
        // 10:40（週六）集保股權分散週摘要入庫（holder_distribution）；開放資料只有最新一週
        job!(
            normal,
            "0 40 10 * * Sat",
            "集保股權分散入庫",
            chip::execute_holder_distributions,
        ),
        // 17:30、20:30（週一至週五）以證交所、櫃買中心官方值補最近 7 天日線的本益比與股價淨值比
        // （櫃買當天資料 17:30 常未公布，20:30 再跑一次）
        job!(
            normal,
            "0 30 17,20 * * Mon-Fri",
            "補齊官方本益比與股價淨值比",
            official_ratio::execute,
        ),
        // 20:50（每月 10–28 日）董監事持股與設質入庫（insider_holding）；同月份重跑只更新有變的列
        job!(
            normal,
            "0 50 20 10-28 * *",
            "董監持股入庫",
            chip::execute_insider_holdings,
        ),
        // 22:05（週一至週五）重建 ETF 專區快照，帶進 21:50 入庫的外資買賣超
        job!(
            normal,
            "0 5 22 * * Mon-Fri",
            "重建 ETF 專區快照",
            etf_zone::refresh_snapshot,
        ),
        // 22:10（週一至週五）策略換股通知：依 .env 的 STRATEGY_ALERT_QUERY 跑條件選股回測，
        // 今天是換股日才發；排在 21:50 籌碼入庫之後，外資／投信連續買超條件才看得到今天。
        job!(
            normal,
            "0 10 22 * * Mon-Fri",
            "策略換股通知",
            event::taiwan_stock::strategy_alert::execute,
        ),
        // 22:00 外資持股狀態
        job!(
            normal,
            "0 0 22 * * *",
            "更新外資持股比例與狀態",
            qualified_foreign_institutional_investor::execute,
        ),
        // 23:00（週一至週五）檢查大盤指數、籌碼與外資持股是否寫入，缺漏就重跑並告警；
        // 另重跑一次官方本益比補寫，有補到才告警（排程失敗時當天的資料不會一直缺著）。
        job!(
            normal,
            "0 0 23 * * Mon-Fri",
            "檢查盤後籌碼與外資資料完整性",
            event::taiwan_stock::closing::ensure_after_hours,
        ),
        // 每 10 分鐘檢查 pi 的供電／降頻、可用記憶體與 swap、SD 卡寫入耗時，異常時告警（同類 6 小時一次）；
        // 一天 144 輪，task.begin／task.done 記 DEBUG。非 pi 讀不到的項目略過。
        job!(
            quiet,
            "0 */10 * * * *",
            "主機健康檢查",
            event::host_health::execute,
        ),
        // 23:30 記一行當日主機健康彙總（fsync 最大耗時、最低可用記憶體、swap 最高使用率、目前與開機以來是否降頻、部署期間次數）
        job!(
            normal,
            "0 30 23 * * *",
            "主機健康彙總",
            event::host_health::summarize,
        ),
    ]
}
