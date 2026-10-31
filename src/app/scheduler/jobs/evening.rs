//! 15:00 起的盤後：收盤行情、資料完整性檢查、籌碼入庫與持股通知。
use crate::app::backfill::{
    chip, dividend, etf_zone, official_ratio, qualified_foreign_institutional_investor,
};
use crate::app::event;

use super::JobDefinition;

/// 本時段的排程，依執行時間排序。
pub(super) fn definitions() -> Vec<JobDefinition> {
    vec![
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
        // 17:30、20:30（週一至週五）以證交所、櫃買中心官方值補最近 7 天日線的本益比與股價淨值比
        // （櫃買當天資料 17:30 常未公布，20:30 再跑一次）
        job!(
            normal,
            "0 30 17,20 * * Mon-Fri",
            "補齊官方本益比與股價淨值比",
            official_ratio::execute,
        ),
        // 17:45（週一至週五）持有 ETF 剛轉為高溢價、剛好連續折價滿 5 天，或單日申贖達 ±3% 就通知，排在 17:30 淨值入庫之後。
        job!(
            normal,
            "0 45 17 * * Mon-Fri",
            "通知持有 ETF 折溢價與申贖",
            event::taiwan_stock::etf_premium_alert::execute,
        ),
        // 19:15（週一至週五）通知持股的重要技術訊號與相對強度轉折；排在 19:00 收盤完整性檢查之後，
        // 若有重跑收盤匯總，訊號也已重新計算。
        job!(
            normal,
            "0 15 19 * * Mon-Fri",
            "通知持股技術訊號",
            event::taiwan_stock::technical_alert::execute,
        ),
        // 19:20（週一至週五）全市場區間突破通知：依成交值列前 15 檔，持股另外標示；排在持股訊號通知之後。
        job!(
            normal,
            "0 20 19 * * Mon-Fri",
            "全市場區間突破通知",
            event::taiwan_stock::breakout_alert::execute,
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
        // 20:50（每月 10–28 日）董監事持股與設質入庫（insider_holding）；同月份重跑只更新有變的列
        job!(
            normal,
            "0 50 20 10-28 * *",
            "董監持股入庫",
            chip::execute_insider_holdings,
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
        // 22:00 外資持股狀態
        job!(
            normal,
            "0 0 22 * * *",
            "更新外資持股比例與狀態",
            qualified_foreign_institutional_investor::execute,
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
        // 23:00（週一至週五）檢查大盤指數、籌碼與外資持股是否寫入，缺漏就重跑並告警；
        // 另重跑一次官方本益比補寫，有補到才告警（排程失敗時當天的資料不會一直缺著）。
        job!(
            normal,
            "0 0 23 * * Mon-Fri",
            "檢查盤後籌碼與外資資料完整性",
            event::taiwan_stock::closing::ensure_after_hours,
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
