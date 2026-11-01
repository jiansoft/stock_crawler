//! # 每週資料健康檢查
//!
//! 來源改版或抓取失敗常常不會報錯：MOPS 舊頁 404 後外資持股靜默停更、上櫃漲跌負號
//! 遺失多年、補值列漏填年月日、回補過去的行情後均線沒有重算……都是事後才發現。
//! 這裡每週把這類「不會自己報錯」的狀況量一次，以 Telegram 發一則週報。
//!
//! ## 檢查項目（近 7 天）
//!
//! 1. 交易日完整：非休市的平日要有日報價，上市、上櫃有成交檔數不能低於近期的一半。
//! 2. 均線：前 400 天已有 5 筆以上報價，5 日均線卻是 0。
//! 3. 漲跌幅：與「漲跌 ÷ 參考價」不符。
//! 4. 年月日欄位：與日期不符。
//! 5. 衍生資料：估價、殖利率排行、市場統計、最後交易日報價、技術訊號、相對強度、ETF 績效與快照
//!    要更新到最新交易日；
//!    CAGR、外資持股、三大法人與融資融券（`chip_daily`）、持股主力進出（`broker_flow`）允許落後一個交易日；
//!    未來報酬的 5 日報酬應到第 5 個交易日前的基準日。
//! 6. 月營收：每月 15 日之後要有上個月的資料。
//! 7. 股利：近五年過期的「尚未公布」佔位列、年度合計與各期明細加總不符。
//! 8. 本益比／淨值比：某天某市場為 0 的比例超過門檻（官方補值排程沒跑或來源改版）。
//!    虧損股的本益比本來就是 0（上櫃約 26%），所以本益比的門檻較寬；今天還沒補值，不列入。
//! 9. 未來報酬（整張表）：基準日有兩成以上「長天期有值、短天期是空的」，代表回補的結束日沒涵蓋到
//!    （2026-10 曾發現 2022 年的 5／20／60 日報酬缺了 45 萬列，訊號績效默默少了樣本）。
//! 10. 籌碼連續天數：`chip_daily` 的連續買賣超天數是 NULL（每日入庫後的重算失敗）。
//! 11. 財報落後：官方季報公布滿 [`LAGGING_MIN_AGE_DAYS`] 天（一年內）的上市櫃股票，Yahoo／BigGo 單季
//!     損益表還沒有那一季（季報期優先採集也沒追上，選股、回測與持股健檢會用到舊一季）。
//! 12. 週／月資料：千張大戶（`holder_distribution`）超過 [`HOLDER_MAX_AGE_DAYS`] 天沒更新、董監持股
//!     （`insider_holding`）的最新月份早於 [`INSIDER_MAX_AGE_DAYS`] 天前那個月。兩者來源都只有最新
//!     一期，停更幾期就永遠補不回來。同一個排程寫入的集保全部持股分級（`holder_distribution_level`）
//!     最新週要跟上千張大戶、列數不少於 [`HOLDER_LEVEL_MIN_ROWS`]，千張大戶最新週的 400 張欄位不能是空值；
//!     這一項正常時也附上級距的最新週與列數。
//! 13. ETF 資料：交易日的 ETF 淨值（`etf_daily_nav`）比近期少一成以上；最新交易日以前的正式淨值
//!     有一成以上沒回填；基本資料（`etf_profile`）超過 [`ETF_PROFILE_MAX_AGE_DAYS`] 天沒更新。
//!     MIS 只給當天，漏抓那天要靠週六的 MoneyDJ 補缺日，太久沒發現就超出補的範圍。
//!
//! 全部正常時也會發一則簡短的週報，用來確認檢查本身有在跑。
//! 最後附兩行參考資訊（不算檢查項目）：本週慢查詢（檔案日誌）與資料庫端耗時（`pg_stat_statements`）。

use std::fmt::Write;

use anyhow::Result;
use chrono::{Datelike, Days, Local, NaiveDate, Weekday};

use super::closing;
use crate::{
    core::{alert, util::text},
    domain::{
        financial::{repository::FinancialReportRepository, statement::LaggingStatement},
        health::{
            DataHealthRepository, DataHealthSnapshot, ForwardReturnGap, StatementStats,
            ValuationRatioZeroCount,
        },
    },
    infra::{
        crawler::twse,
        database::repository::{
            data_health::PgDataHealthRepository, financial_report::PgFinancialReportRepository,
        },
    },
};

mod abnormal_prices;
mod db_statements;
mod db_uptime;
mod host_summary;
mod rules;
mod slow_queries;

// 規則檢核搬到子模組；週報組裝與排程入口留在這裡。
use self::abnormal_prices::{abnormal_price_line, weekly_abnormal_prices};
use self::host_summary::{host_summary_line, weekly_host_days};
use self::rules::*;
use self::slow_queries::{LOG_DIR, slow_query_line, weekly_slow_queries};
/// 檢查最近幾個日曆日（含今天）。
const CHECK_DAYS: u64 = 7;

/// 成交檔數的比較基準再往前看幾個日曆日。
const BASELINE_DAYS: u64 = 21;

/// 月營收在每月這一天之後應已有上個月的資料（法定申報期限為 10 日）。
const REVENUE_DUE_DAY: u32 = 15;

/// 千張大戶週摘要最多可以幾天沒更新：週六 10:40 抓上週五的資料，週五休市時資料日更早，
/// 寬限到 9 天才算漏了一週。
const HOLDER_MAX_AGE_DAYS: u64 = 9;

/// 集保全部持股分級最新一週至少要有幾列：每檔 17 列，上市櫃與興櫃約 4,100 檔、69,700 列，
/// 少了一成多代表只寫進一部分。
const HOLDER_LEVEL_MIN_ROWS: i64 = 60_000;

/// 董監持股的最新月份不能早於這麼多天前那個月：每月 10～28 日抓上個月的資料，月中前還沒公布
/// 時最新只到前兩個月，55 天涵蓋這段空窗。
const INSIDER_MAX_AGE_DAYS: u64 = 55;

/// ETF 基本資料最多可以幾天沒更新：週六 12:10 更新，健康週報排在同一天 11:00，
/// 正常時剛好差 7 天，寬限到 9 天才算漏了一週。
const ETF_PROFILE_MAX_AGE_DAYS: u64 = 9;

/// 某天的 ETF 淨值列數少於近期最多列數的這個百分比就算漏抓。
const ETF_NAV_MIN_PERCENT: i64 = 90;

/// 某天沒有正式淨值的 ETF 超過這個百分比就算回填失敗（不公布正式淨值的只有零星幾檔）。
const ETF_MISSING_NAV_LIMIT_PERCENT: i64 = 10;

/// 停更的 ETF 基本資料最多列出幾檔，其餘只計數。
const STALE_ETF_PROFILES_LISTED: usize = 10;

/// 某天某市場的股價淨值比為 0 超過這個百分比就算異常（正常約 0～3%）。
const PB_ZERO_LIMIT_PERCENT: i64 = 20;

/// 某天某市場的本益比為 0 超過這個百分比就算異常（虧損股占上櫃約 26%、上市約 22%）。
const PE_ZERO_LIMIT_PERCENT: i64 = 60;

/// 未來報酬缺漏最多列出幾個基準日，其餘只計數。
const FORWARD_RETURN_GAP_LISTED: usize = 5;

/// 官方季報公布滿這麼多天、損益表仍沒有那一季才算落後（Yahoo 通常 1～2 天內更新）。
const LAGGING_MIN_AGE_DAYS: i32 = 7;

/// 財報落後只看一年內公布的官方季報。
const LAGGING_LOOKBACK_DAYS: i32 = 365;

/// 財報落後最多列出幾檔，其餘只計數。
const LAGGING_LISTED: usize = 10;

/// 單一檢查項目的結果。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Check {
    /// 項目名稱。
    name: &'static str,
    /// 發現的問題，空的代表正常。
    problems: Vec<String>,
    /// 附在項目名稱後的量測數字（例如集保級距的最新週），正常時也顯示。
    detail: Option<String>,
}

impl Check {
    fn new(name: &'static str, problems: Vec<String>) -> Self {
        Self {
            name,
            problems,
            detail: None,
        }
    }

    fn with_detail(mut self, detail: Option<String>) -> Self {
        self.detail = detail;
        self
    }

    /// 單一數字超過 0 就算異常的項目。
    fn count(name: &'static str, count: i64, describe: impl FnOnce(i64) -> String) -> Self {
        let problems = if count > 0 {
            vec![describe(count)]
        } else {
            Vec::new()
        };
        Self::new(name, problems)
    }
}

/// 排程入口：量測近 7 天的資料狀況並發送週報。
pub async fn execute() -> Result<()> {
    let (message, statements) = build(Local::now().date_naive()).await?;
    alert::send_message(&message).await;
    // 發送後才存資料庫端累計值，下週的「資料庫端耗時」從這裡算差額。
    if let Some(statements) = statements {
        db_statements::save(db_statements::SNAPSHOT_KEY, &statements).await;
    }
    Ok(())
}

/// 量測截至 `today` 的近 7 天資料狀況，回傳週報內容（不發送，也不更新資料庫端耗時的起點）。
pub async fn build_report(today: NaiveDate) -> Result<String> {
    build(today).await.map(|(message, _)| message)
}

/// 週報內容與這次讀到的資料庫端累計值。
async fn build(today: NaiveDate) -> Result<(String, Option<StatementStats>)> {
    let repo = PgDataHealthRepository::new();
    let from = today - Days::new(CHECK_DAYS - 1);
    let baseline_from = from - Days::new(BASELINE_DAYS);

    let holidays = fetch_holidays(from, today).await;
    let profile_due = today - Days::new(ETF_PROFILE_MAX_AGE_DAYS);
    let snapshot = repo
        .fetch_snapshot(baseline_from, from, today, today.year(), profile_due)
        .await?;

    let mut checks = evaluate(&snapshot, from, today, holidays.as_deref());
    checks.push(lagging_check(&PgFinancialReportRepository::new()).await);
    let problems = checks
        .iter()
        .filter(|check| !check.problems.is_empty())
        .count();
    tracing::info!(
        "資料健康檢查結束: checks={}, problems={problems}",
        checks.len()
    );
    let slow = weekly_slow_queries(std::path::Path::new(LOG_DIR), from, today);
    let abnormal = weekly_abnormal_prices(std::path::Path::new(LOG_DIR), from, today);
    let host_days = weekly_host_days(std::path::Path::new(LOG_DIR), from, today);
    let (statement_line, statements) = db_statements::weekly_statements(&repo).await;
    let uptime_line = db_uptime::weekly_uptime_line(from).await;
    let message = format!(
        "{}
{}
{}
{}
{}",
        with_slow_queries(report(from, today, &checks), &slow),
        text::escape_markdown_v2(abnormal_price_line(&abnormal)),
        text::escape_markdown_v2(host_summary_line(&host_days)),
        text::escape_markdown_v2(&uptime_line),
        text::escape_markdown_v2(&statement_line)
    );
    Ok((message, statements))
}

/// 週報最後附上本週慢查詢（不算檢查項目，只供參考）。
fn with_slow_queries(
    message: String,
    slow: &crate::core::logging::slow_query::SlowQueryTally,
) -> String {
    format!(
        "{message}\n{}",
        text::escape_markdown_v2(slow_query_line(slow))
    )
}

/// 取得期間內的休市日；任一年度抓取失敗回傳 `None`，交易日完整性改為只看成交檔數。
async fn fetch_holidays(from: NaiveDate, to: NaiveDate) -> Option<Vec<NaiveDate>> {
    let mut holidays = Vec::new();
    for year in from.year()..=to.year() {
        match twse::holiday_schedule::visit(year).await {
            Ok(schedule) => holidays.extend(schedule.into_iter().map(|holiday| holiday.date)),
            Err(why) => {
                tracing::warn!("取得 {year} 年休市日失敗，略過缺漏交易日檢查: {why:?}");
                return None;
            }
        }
    }
    Some(holidays)
}

/// 依量測值判斷每個項目是否正常。`today` 當天還沒收盤，不列入應有交易日。
fn evaluate(
    snapshot: &DataHealthSnapshot,
    from: NaiveDate,
    today: NaiveDate,
    holidays: Option<&[NaiveDate]>,
) -> Vec<Check> {
    let trading_days = trading_days_desc(snapshot);
    vec![
        Check::new(
            "交易日完整",
            trading_day_problems(snapshot, from, today, holidays),
        ),
        Check::count("均線", snapshot.missing_moving_averages, |count| {
            format!("{count} 列 5 日均線是 0（請跑 test_recalculate_moving_averages）")
        }),
        Check::count("漲跌幅", snapshot.change_range_mismatches, |count| {
            format!("{count} 列與「漲跌 ÷ 參考價」不符")
        }),
        Check::count("年月日欄位", snapshot.misdated_rows, |count| {
            format!("{count} 列 year／month／day 與日期不符")
        }),
        Check::count("籌碼連續天數", snapshot.chip_streak_missing, |count| {
            format!("{count} 列連續買賣超天數未計算（請跑 test_backfill_chip_streaks）")
        }),
        Check::new("衍生資料", derived_table_problems(snapshot, &trading_days)),
        Check::new(
            "月營收",
            revenue_problems(snapshot.latest_revenue_month, today),
        ),
        Check::new("股利", dividend_problems(snapshot)),
        Check::new("週／月資料", periodic_table_problems(snapshot, today))
            .with_detail(holder_level_detail(snapshot)),
        Check::new(
            "本益比／淨值比",
            valuation_ratio_problems(&snapshot.valuation_ratio_zeros, today),
        ),
        Check::new(
            "未來報酬",
            [
                forward_return_gap_problems(&snapshot.forward_return_gaps),
                market_forward_return_drift_problems(&snapshot.market_forward_return_drift),
            ]
            .concat(),
        ),
        Check::new(
            "ETF 資料",
            etf_problems(snapshot, from, today, &trading_days),
        ),
    ]
}

fn report(from: NaiveDate, to: NaiveDate, checks: &[Check]) -> String {
    let problems = checks
        .iter()
        .filter(|check| !check.problems.is_empty())
        .count();
    let mut message = String::new();
    let _ = writeln!(&mut message, "資料健康週報 {from}～{to}");
    if problems == 0 {
        let _ = writeln!(&mut message, "全部 {} 項正常", checks.len());
    } else {
        let _ = writeln!(&mut message, "{problems} 項需要處理");
    }
    for check in checks {
        let mark = if check.problems.is_empty() {
            "✅"
        } else {
            "⚠️"
        };
        match &check.detail {
            Some(detail) => {
                let _ = writeln!(&mut message, "{mark} {}（{detail}）", check.name);
            }
            None => {
                let _ = writeln!(&mut message, "{mark} {}", check.name);
            }
        }
        for problem in &check.problems {
            let _ = writeln!(&mut message, "  • {problem}");
        }
    }
    text::escape_markdown_v2(message.trim_end())
}

#[cfg(test)]
mod tests;
