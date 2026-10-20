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
//! 5. 衍生資料：估價、殖利率排行、市場統計、最後交易日報價、技術訊號、相對強度要更新到最新交易日；
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
//!
//! 全部正常時也會發一則簡短的週報，用來確認檢查本身有在跑。

use std::fmt::Write;

use anyhow::Result;
use chrono::{Datelike, Days, Local, NaiveDate, Weekday};

use super::closing;
use crate::{
    core::{alert, util::text},
    domain::{
        financial::{repository::FinancialReportRepository, statement::LaggingStatement},
        health::{
            DataHealthRepository, DataHealthSnapshot, ForwardReturnGap, ValuationRatioZeroCount,
        },
    },
    infra::{
        crawler::twse,
        database::repository::{
            data_health::PgDataHealthRepository, financial_report::PgFinancialReportRepository,
        },
    },
};

/// 檢查最近幾個日曆日（含今天）。
const CHECK_DAYS: u64 = 7;

/// 成交檔數的比較基準再往前看幾個日曆日。
const BASELINE_DAYS: u64 = 21;

/// 月營收在每月這一天之後應已有上個月的資料（法定申報期限為 10 日）。
const REVENUE_DUE_DAY: u32 = 15;

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
}

impl Check {
    fn new(name: &'static str, problems: Vec<String>) -> Self {
        Self { name, problems }
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
    let message = build_report(Local::now().date_naive()).await?;
    alert::send_message(&message).await;
    Ok(())
}

/// 量測截至 `today` 的近 7 天資料狀況，回傳週報內容（不發送）。
pub async fn build_report(today: NaiveDate) -> Result<String> {
    let from = today - Days::new(CHECK_DAYS - 1);
    let baseline_from = from - Days::new(BASELINE_DAYS);

    let holidays = fetch_holidays(from, today).await;
    let snapshot = PgDataHealthRepository::new()
        .fetch_snapshot(baseline_from, from, today, today.year())
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
    Ok(report(from, today, &checks))
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
        Check::new(
            "本益比／淨值比",
            valuation_ratio_problems(&snapshot.valuation_ratio_zeros, today),
        ),
        Check::new(
            "未來報酬",
            forward_return_gap_problems(&snapshot.forward_return_gaps),
        ),
    ]
}

/// 有成交紀錄的交易日，由新到舊。
fn trading_days_desc(snapshot: &DataHealthSnapshot) -> Vec<NaiveDate> {
    let mut days: Vec<NaiveDate> = snapshot
        .market_counts
        .iter()
        .filter(|count| count.traded > 0)
        .map(|count| count.date)
        .collect();
    days.sort_unstable_by(|a, b| b.cmp(a));
    days.dedup();
    days
}

/// 檢查期間內每個應有交易日：整天沒有日報價，或某個市場有成交的檔數明顯偏少。
fn trading_day_problems(
    snapshot: &DataHealthSnapshot,
    from: NaiveDate,
    today: NaiveDate,
    holidays: Option<&[NaiveDate]>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for date in from.iter_days().take_while(|date| *date < today) {
        let quoted = snapshot.quoted_dates.contains(&date);
        if !quoted {
            let expected = !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
                && holidays.is_some_and(|holidays| !holidays.contains(&date));
            if expected {
                problems.push(format!("{date} 整天沒有日報價"));
            }
            continue;
        }
        for shortfall in closing::shortfalls(date, &snapshot.market_counts) {
            problems.push(format!(
                "{date} {} 只有 {} 檔有成交（近期 {} 檔）",
                shortfall.market, shortfall.traded, shortfall.baseline
            ));
        }
    }
    if holidays.is_none() {
        problems.push("休市日清單抓取失敗，未檢查整天缺漏".to_string());
    }
    problems
}

/// 衍生資料表允許落後最新交易日幾個交易日。
fn allowed_lag(table: &str) -> usize {
    match table {
        // 05:40 才算前一個交易日的 CAGR；22:00 才抓當天的外資持股；21:50 才入庫當天的籌碼；
        // 20:40 才抓當天的主力進出。
        "stock_cagr" | "qfii_history" | "chip_daily" | "broker_flow" => 1,
        // 基準日之後要再過 5 個交易日才算得出 5 日報酬。
        "daily_forward_return" => 5,
        _ => 0,
    }
}

/// 衍生資料表沒有更新到應有的交易日。
fn derived_table_problems(
    snapshot: &DataHealthSnapshot,
    trading_days: &[NaiveDate],
) -> Vec<String> {
    snapshot
        .derived_tables
        .iter()
        .filter_map(|derived| {
            let expected = trading_days
                .get(allowed_lag(derived.table))
                .or(trading_days.last())?;
            match derived.latest {
                Some(latest) if latest >= *expected => None,
                Some(latest) => Some(format!(
                    "{} 最新 {latest}，應至少到 {expected}",
                    derived.table
                )),
                None => Some(format!("{} 沒有任何資料", derived.table)),
            }
        })
        .collect()
}

/// 每月 15 日之後仍沒有上個月的營收。
fn revenue_problems(latest_month: Option<i64>, today: NaiveDate) -> Vec<String> {
    if today.day() <= REVENUE_DUE_DAY {
        return Vec::new();
    }
    let previous = today
        .with_day(1)
        .and_then(|first| first.pred_opt())
        .map(|date| i64::from(date.year()) * 100 + i64::from(date.month()));
    match (latest_month, previous) {
        (Some(latest), Some(expected)) if latest >= expected => Vec::new(),
        (Some(latest), Some(expected)) => {
            vec![format!("最新只到 {latest}，應已有 {expected}")]
        }
        (None, _) => vec!["沒有任何月營收資料".to_string()],
        (_, None) => Vec::new(),
    }
}

/// 過期的「尚未公布」佔位列與年度合計不符。
fn dividend_problems(snapshot: &DataHealthSnapshot) -> Vec<String> {
    let mut problems = Vec::new();
    if snapshot.stale_dividend_placeholders > 0 {
        problems.push(format!(
            "近五年有 {} 列發放年度已過，除權息日仍是「尚未公布」",
            snapshot.stale_dividend_placeholders
        ));
    }
    if snapshot.dividend_total_mismatches > 0 {
        problems.push(format!(
            "{} 組年度合計與各期明細加總不符",
            snapshot.dividend_total_mismatches
        ));
    }
    problems
}

/// 本益比或淨值比為 0 的比例超過門檻的交易日與市場；`today` 還沒補值，不列入。
fn valuation_ratio_problems(zeros: &[ValuationRatioZeroCount], today: NaiveDate) -> Vec<String> {
    let mut problems = Vec::new();
    for zero in zeros
        .iter()
        .filter(|zero| zero.date < today && zero.rows > 0)
    {
        let market = if zero.market_id == 2 {
            "上市"
        } else {
            "上櫃"
        };
        for (label, count, limit) in [
            ("本益比", zero.pe_zero, PE_ZERO_LIMIT_PERCENT),
            ("淨值比", zero.pb_zero, PB_ZERO_LIMIT_PERCENT),
        ] {
            if count * 100 > zero.rows * limit {
                problems.push(format!(
                    "{} {market} {label}為 0 的有 {count}／{} 檔（請跑 test_backfill_official_valuation_ratios）",
                    zero.date, zero.rows
                ));
            }
        }
    }
    problems
}

/// 未來報酬短天期缺漏的基準日：列出最早的幾天與總天數，並提示重跑的起日。
///
/// 回補以「結束日」逐日計算，從最早缺漏的基準日起重跑到今天就會補齊之後各天期。
fn forward_return_gap_problems(gaps: &[ForwardReturnGap]) -> Vec<String> {
    let (Some(first), Some(last)) = (gaps.first(), gaps.last()) else {
        return Vec::new();
    };
    let listed = gaps
        .iter()
        .take(FORWARD_RETURN_GAP_LISTED)
        .map(|gap| format!("{} {} 列", gap.date, gap.rows))
        .collect::<Vec<_>>()
        .join("、");
    let more = if gaps.len() > FORWARD_RETURN_GAP_LISTED {
        "…"
    } else {
        ""
    };
    vec![format!(
        "{} 個基準日（{}～{}）的短天期報酬缺漏：{listed}{more}；請以 MANUAL_FORWARD_RETURN_FROM={} 重跑 test_backfill_forward_returns",
        gaps.len(),
        first.date,
        last.date,
        first.date
    )]
}

/// 組成 Telegram 週報。
///
/// 通知管道以 MarkdownV2 發送，週報本身沒有格式標記，因此整則跳脫保留字元；
/// 日期的 `-` 沒跳脫時 Telegram 會整則拒收（400）。
/// 財報落後項目：查詢官方季報公布已久、損益表仍沒有那一季的股票。
async fn lagging_check(repo: &impl FinancialReportRepository) -> Check {
    let lagging = match repo
        .fetch_lagging_statements(LAGGING_LOOKBACK_DAYS, LAGGING_MIN_AGE_DAYS)
        .await
    {
        Ok(lagging) => Some(lagging),
        Err(why) => {
            tracing::warn!("查詢財報落後失敗: {why:#}");
            None
        }
    };
    Check::new("財報落後", lagging_problems(lagging.as_deref()))
}

/// 官方季報公布已久、損益表仍沒有那一季的股票；查詢失敗（`None`）本身也是問題。
fn lagging_problems(lagging: Option<&[LaggingStatement]>) -> Vec<String> {
    let Some(lagging) = lagging else {
        return vec!["查詢失敗，見 warn 日誌".to_owned()];
    };
    if lagging.is_empty() {
        return Vec::new();
    }
    let mut listed: Vec<String> = lagging
        .iter()
        .take(LAGGING_LISTED)
        .map(|statement| {
            format!(
                "{} {}{}（{} 公布）",
                statement.stock_symbol,
                statement.fiscal_year,
                statement.quarter,
                statement.published_on.format("%m-%d")
            )
        })
        .collect();
    if lagging.len() > LAGGING_LISTED {
        listed.push(format!("…共 {} 檔", lagging.len()));
    }
    vec![format!(
        "{} 檔官方季報已公布 {LAGGING_MIN_AGE_DAYS} 天以上、損益表還沒有那一季：{}",
        lagging.len(),
        listed.join("、")
    )]
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
        if check.problems.is_empty() {
            let _ = writeln!(&mut message, "✅ {}", check.name);
        } else {
            let _ = writeln!(&mut message, "⚠️ {}", check.name);
            for problem in &check.problems {
                let _ = writeln!(&mut message, "  • {problem}");
            }
        }
    }
    text::escape_markdown_v2(message.trim_end())
}

#[cfg(test)]
mod tests;
