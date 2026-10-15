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
//!    CAGR、外資持股、三大法人與融資融券（`chip_daily`）允許落後一個交易日；
//!    未來報酬的 5 日報酬應到第 5 個交易日前的基準日。
//! 6. 月營收：每月 15 日之後要有上個月的資料。
//! 7. 股利：近五年過期的「尚未公布」佔位列、年度合計與各期明細加總不符。
//! 8. 本益比／淨值比：某天某市場為 0 的比例超過門檻（官方補值排程沒跑或來源改版）。
//!    虧損股的本益比本來就是 0（上櫃約 26%），所以本益比的門檻較寬；今天還沒補值，不列入。
//! 9. 未來報酬（整張表）：基準日有兩成以上「長天期有值、短天期是空的」，代表回補的結束日沒涵蓋到
//!    （2026-10 曾發現 2022 年的 5／20／60 日報酬缺了 45 萬列，訊號績效默默少了樣本）。
//!
//! 全部正常時也會發一則簡短的週報，用來確認檢查本身有在跑。

use std::fmt::Write;

use anyhow::Result;
use chrono::{Datelike, Days, Local, NaiveDate, Weekday};

use super::closing;
use crate::{
    core::{alert, util::text},
    domain::health::{
        DataHealthRepository, DataHealthSnapshot, ForwardReturnGap, ValuationRatioZeroCount,
    },
    infra::{crawler::twse, database::repository::data_health::PgDataHealthRepository},
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

    let checks = evaluate(&snapshot, from, today, holidays.as_deref());
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
        // 05:40 才算前一個交易日的 CAGR；22:00 才抓當天的外資持股；21:50 才入庫當天的籌碼。
        "stock_cagr" | "qfii_history" | "chip_daily" => 1,
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
mod tests {
    use super::*;

    /// 對 `.env` 的資料庫量一次並印出週報，不發送（唯讀）。
    ///
    /// `cargo test app::event::taiwan_stock::data_health::tests::print_report -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn print_report() {
        dotenvy::dotenv().ok();
        let message = build_report(Local::now().date_naive())
            .await
            .expect("量測應成功");
        println!("{message}");
    }
    use crate::domain::{health::DerivedTableLatest, quote::entity::MarketTradedCount};

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).expect("日期應合法")
    }

    fn count(date: NaiveDate, market_id: i32, traded: i64) -> MarketTradedCount {
        MarketTradedCount {
            date,
            market_id,
            traded,
        }
    }

    /// 2026-10-05（一）～10-09（五）都正常、衍生表都到 10-09 的量測值；
    /// 基準期含 10-02（五），未來報酬的 5 日報酬應到這天。
    fn healthy() -> DataHealthSnapshot {
        let days = [day(5), day(6), day(7), day(8), day(9)];
        DataHealthSnapshot {
            market_counts: std::iter::once(day(2))
                .chain(days)
                .flat_map(|date| [count(date, 2, 1_000), count(date, 4, 880)])
                .collect(),
            quoted_dates: days.to_vec(),
            derived_tables: [
                ("estimate", day(9)),
                ("yield_rank", day(9)),
                ("daily_stock_price_stats", day(9)),
                ("last_daily_quotes", day(9)),
                ("stock_cagr", day(8)),
                ("qfii_history", day(8)),
                ("daily_technical_signal", day(9)),
                ("daily_relative_strength", day(9)),
                ("chip_daily", day(8)),
                ("daily_forward_return", day(2)),
            ]
            .into_iter()
            .map(|(table, latest)| DerivedTableLatest {
                table,
                latest: Some(latest),
            })
            .collect(),
            latest_revenue_month: Some(202609),
            ..Default::default()
        }
    }

    fn problems_of<'a>(checks: &'a [Check], name: &str) -> &'a [String] {
        &checks
            .iter()
            .find(|check| check.name == name)
            .expect("應有此項目")
            .problems
    }

    /// 全部正常時每一項都沒有問題，週報只說「全部正常」。
    #[test]
    fn a_healthy_week_reports_no_problems() {
        let checks = evaluate(&healthy(), day(4), day(10), Some(&[]));
        assert!(
            checks.iter().all(|check| check.problems.is_empty()),
            "{checks:?}"
        );

        let message = report(day(4), day(10), &checks);
        assert!(
            message.starts_with("資料健康週報 2026\\-10\\-04～2026\\-10\\-10\n全部 9 項正常"),
            "{message}"
        );
        assert!(!message.contains("⚠️"));
    }

    /// 本益比、淨值比為 0 的比例超過門檻才列出；今天與沒有列數的日子不檢查。
    #[test]
    fn zero_valuation_ratios_over_the_limit_are_flagged() {
        let zero = |d, market_id, rows, pe_zero, pb_zero| ValuationRatioZeroCount {
            date: day(d),
            market_id,
            rows,
            pe_zero,
            pb_zero,
        };
        let mut snapshot = healthy();
        snapshot.valuation_ratio_zeros = vec![
            zero(8, 2, 1_100, 243, 27),
            zero(8, 4, 894, 236, 2),
            zero(9, 2, 1_100, 243, 1_100),
            zero(9, 4, 894, 890, 2),
            zero(10, 4, 894, 894, 894),
            zero(7, 4, 0, 0, 0),
        ];
        let checks = evaluate(&snapshot, day(4), day(10), Some(&[]));
        assert_eq!(
            problems_of(&checks, "本益比／淨值比"),
            [
                "2026-10-09 上市 淨值比為 0 的有 1100／1100 檔（請跑 test_backfill_official_valuation_ratios）",
                "2026-10-09 上櫃 本益比為 0 的有 890／894 檔（請跑 test_backfill_official_valuation_ratios）",
            ]
        );
    }

    /// 平日整天沒有日報價要列出；休市日與週末不算。
    #[test]
    fn a_missing_weekday_is_flagged_unless_it_is_a_holiday() {
        let mut snapshot = healthy();
        snapshot
            .quoted_dates
            .retain(|date| *date != day(7) && *date != day(8));
        snapshot
            .market_counts
            .retain(|count| count.date != day(7) && count.date != day(8));

        let checks = evaluate(&snapshot, day(4), day(10), Some(&[day(8)]));

        assert_eq!(
            problems_of(&checks, "交易日完整"),
            ["2026-10-07 整天沒有日報價"]
        );
    }

    /// 只剩零量補值列的市場（有日報價、成交檔數 0）也要列出。
    #[test]
    fn a_market_with_only_fill_rows_is_flagged() {
        let mut snapshot = healthy();
        snapshot
            .market_counts
            .retain(|count| !(count.date == day(9) && count.market_id == 4));

        let checks = evaluate(&snapshot, day(4), day(10), Some(&[]));

        assert_eq!(
            problems_of(&checks, "交易日完整"),
            ["2026-10-09 上櫃 只有 0 檔有成交（近期 880 檔）"]
        );
    }

    /// 休市日抓不到時不猜哪天該開市，但要在週報註明沒檢查。
    #[test]
    fn missing_holidays_skip_the_whole_day_check() {
        let mut snapshot = healthy();
        snapshot.quoted_dates.retain(|date| *date != day(7));
        snapshot.market_counts.retain(|count| count.date != day(7));

        let checks = evaluate(&snapshot, day(4), day(10), None);

        assert_eq!(
            problems_of(&checks, "交易日完整"),
            ["休市日清單抓取失敗，未檢查整天缺漏"]
        );
    }

    /// 收盤後的衍生表必須到最新交易日；CAGR、外資持股與籌碼可以晚一天，再晚就算落後；
    /// 未來報酬看第 5 個交易日前的基準日。
    #[test]
    fn derived_tables_must_keep_up_with_the_latest_trading_day() {
        let mut snapshot = healthy();
        for derived in &mut snapshot.derived_tables {
            match derived.table {
                "estimate" => derived.latest = Some(day(8)),
                "stock_cagr" => derived.latest = Some(day(7)),
                "yield_rank" => derived.latest = None,
                "daily_relative_strength" => derived.latest = Some(day(8)),
                "chip_daily" => derived.latest = Some(day(7)),
                "daily_forward_return" => derived.latest = Some(day(1)),
                _ => {}
            }
        }

        let checks = evaluate(&snapshot, day(4), day(10), Some(&[]));

        assert_eq!(
            problems_of(&checks, "衍生資料"),
            [
                "estimate 最新 2026-10-08，應至少到 2026-10-09",
                "yield_rank 沒有任何資料",
                "stock_cagr 最新 2026-10-07，應至少到 2026-10-08",
                "daily_relative_strength 最新 2026-10-08，應至少到 2026-10-09",
                "chip_daily 最新 2026-10-07，應至少到 2026-10-08",
                "daily_forward_return 最新 2026-10-01，應至少到 2026-10-02",
            ]
        );
    }

    /// 未來報酬缺漏列出最早的 5 個基準日與總數，沒有缺漏時不列。
    #[test]
    fn forward_return_gaps_list_the_first_dates() {
        assert!(forward_return_gap_problems(&[]).is_empty());
        let gaps: Vec<ForwardReturnGap> = (1..=7)
            .map(|d| ForwardReturnGap {
                date: NaiveDate::from_ymd_opt(2022, 10, d).expect("日期應合法"),
                rows: 2_000 + i64::from(d),
            })
            .collect();
        let problems = forward_return_gap_problems(&gaps);
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0].starts_with(
                "7 個基準日（2022-10-01～2022-10-07）的短天期報酬缺漏：2022-10-01 2001 列、"
            ),
            "{problems:?}"
        );
        assert!(
            problems[0].ends_with(
                "2022-10-05 2005 列…；請以 MANUAL_FORWARD_RETURN_FROM=2022-10-01 重跑 test_backfill_forward_returns"
            ),
            "{problems:?}"
        );
        let single = forward_return_gap_problems(&gaps[..1]);
        assert!(single[0].contains("：2022-10-01 2001 列；"), "{single:?}");
    }

    /// 15 日之前不要求上個月的營收；之後就要有。
    #[test]
    fn revenue_is_due_after_the_fifteenth() {
        assert!(revenue_problems(Some(202608), day(15)).is_empty());
        assert_eq!(
            revenue_problems(Some(202608), day(16)),
            ["最新只到 202608，應已有 202609"]
        );
        assert!(revenue_problems(Some(202609), day(16)).is_empty());
        // 一月要看前一年十二月。
        let january = NaiveDate::from_ymd_opt(2027, 1, 20).expect("日期應合法");
        assert_eq!(
            revenue_problems(Some(202611), january),
            ["最新只到 202611，應已有 202612"]
        );
    }

    /// 計數類項目大於 0 才列出，週報標出需要處理的項目數。
    #[test]
    fn counted_problems_are_listed_in_the_report() {
        let mut snapshot = healthy();
        snapshot.missing_moving_averages = 12;
        snapshot.stale_dividend_placeholders = 3;
        snapshot.dividend_total_mismatches = 1;

        let checks = evaluate(&snapshot, day(4), day(10), Some(&[]));
        let message = report(day(4), day(10), &checks);

        assert!(message.contains("2 項需要處理"), "{message}");
        assert!(
            message.contains("⚠️ 均線\n  • 12 列 5 日均線是 0"),
            "{message}"
        );
        assert!(
            message.contains(
                "⚠️ 股利\n  • 近五年有 3 列發放年度已過，除權息日仍是「尚未公布」\n  • 1 組年度合計與各期明細加總不符"
            ),
            "{message}"
        );
        assert!(message.contains("✅ 漲跌幅"), "{message}");
        // MarkdownV2 保留字元都要跳脫，否則 Telegram 會整則拒收。
        assert!(
            message.contains("test\\_recalculate\\_moving\\_averages"),
            "{message}"
        );
    }
}
