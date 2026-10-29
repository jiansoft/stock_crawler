//! 資料健康週報的各項規則檢核：交易日缺漏、衍生表落後、定期表、股利、估值比率、未來報酬缺口、ETF 與落後檢查。

use super::*;

/// 有成交紀錄的交易日，由新到舊。
pub(super) fn trading_days_desc(snapshot: &DataHealthSnapshot) -> Vec<NaiveDate> {
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
pub(super) fn trading_day_problems(
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
pub(super) fn allowed_lag(table: &str) -> usize {
    match table {
        // 05:40 才算前一個交易日的 CAGR；22:00 才抓當天的外資持股；21:50 才入庫當天的籌碼；
        // 20:40 才抓當天的主力進出。
        "stock_cagr" | "qfii_history" | "chip_daily" | "broker_flow" => 1,
        // 基準日之後要再過 5 個交易日才算得出 5 日報酬。
        "daily_forward_return" | "daily_market_forward_return" => 5,
        _ => 0,
    }
}

/// 衍生資料表沒有更新到應有的交易日。
pub(super) fn derived_table_problems(
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
pub(super) fn revenue_problems(latest_month: Option<i64>, today: NaiveDate) -> Vec<String> {
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

/// 千張大戶與董監持股是否停更；沒有任何資料也列出。千張大戶有資料時再看集保級距與 400 張欄位
/// （[`holder_level_problems`]）。
pub(super) fn periodic_table_problems(
    snapshot: &DataHealthSnapshot,
    today: NaiveDate,
) -> Vec<String> {
    let mut problems = Vec::new();
    let holder_due = today.checked_sub_days(Days::new(HOLDER_MAX_AGE_DAYS));
    match (snapshot.latest_holder_week, holder_due) {
        (None, _) => problems.push("holder_distribution 沒有任何資料".to_string()),
        (Some(latest), Some(due)) if latest < due => problems.push(format!(
            "holder_distribution 最新 {latest}，已超過 {HOLDER_MAX_AGE_DAYS} 天沒更新"
        )),
        _ => {}
    }
    if let Some(holder_week) = snapshot.latest_holder_week {
        problems.extend(holder_level_problems(snapshot, holder_week));
    }
    let insider_due = today
        .checked_sub_days(Days::new(INSIDER_MAX_AGE_DAYS))
        .and_then(|date| date.with_day(1));
    match (snapshot.latest_insider_month, insider_due) {
        (None, _) => problems.push("insider_holding 沒有任何資料".to_string()),
        (Some(latest), Some(due)) if latest < due => problems.push(format!(
            "insider_holding 最新 {}，應已有 {}",
            latest.format("%Y-%m"),
            due.format("%Y-%m")
        )),
        _ => {}
    }
    problems
}

/// 集保全部持股分級與 400 張欄位：兩者和千張大戶由同一個週六排程寫入，千張大戶照常更新時
/// 這兩項沒寫進去也不會報錯。級距最新週落後千張大戶、最新週列數少於 [`HOLDER_LEVEL_MIN_ROWS`]、
/// 千張大戶最新週有 400 張空值（集保來源每檔都有第 12～15 級），都列出。
fn holder_level_problems(snapshot: &DataHealthSnapshot, holder_week: NaiveDate) -> Vec<String> {
    let mut problems = Vec::new();
    match snapshot.latest_holder_level_week {
        None => problems.push("holder_distribution_level 沒有任何資料".to_string()),
        Some(level_week) => {
            if level_week < holder_week {
                problems.push(format!(
                    "holder_distribution_level 最新 {level_week}，落後 holder_distribution（{holder_week}）"
                ));
            }
            if snapshot.holder_level_rows < HOLDER_LEVEL_MIN_ROWS {
                problems.push(format!(
                    "holder_distribution_level {level_week} 只有 {} 列，應至少 {HOLDER_LEVEL_MIN_ROWS} 列",
                    snapshot.holder_level_rows
                ));
            }
        }
    }
    if snapshot.holders_400_missing > 0 {
        problems.push(format!(
            "holder_distribution {holder_week} 有 {} 檔 holders_400 是空值",
            snapshot.holders_400_missing
        ));
    }
    problems
}

/// 週／月資料項目在週報上附的集保級距數字（最新週、列數與 400 張空值檔數），讓正常時也看得到
/// 這週有沒有寫入；沒有級距資料時不附。
pub(super) fn holder_level_detail(snapshot: &DataHealthSnapshot) -> Option<String> {
    snapshot.latest_holder_level_week.map(|week| {
        format!(
            "集保級距 {week} {} 列、400 張空值 {} 檔",
            snapshot.holder_level_rows, snapshot.holders_400_missing
        )
    })
}

/// 過期的「尚未公布」佔位列與年度合計不符。
pub(super) fn dividend_problems(snapshot: &DataHealthSnapshot) -> Vec<String> {
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
pub(super) fn valuation_ratio_problems(
    zeros: &[ValuationRatioZeroCount],
    today: NaiveDate,
) -> Vec<String> {
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

/// 大盤未來報酬彙總與明細對不上的基準日：列出最早的幾天與總天數，並提示重算的期間。
pub(super) fn market_forward_return_drift_problems(dates: &[NaiveDate]) -> Vec<String> {
    let (Some(first), Some(last)) = (dates.first(), dates.last()) else {
        return Vec::new();
    };
    let listed = dates
        .iter()
        .take(FORWARD_RETURN_GAP_LISTED)
        .map(NaiveDate::to_string)
        .collect::<Vec<_>>()
        .join("、");
    let more = if dates.len() > FORWARD_RETURN_GAP_LISTED {
        "…"
    } else {
        ""
    };
    vec![format!(
        "大盤未來報酬彙總有 {} 個基準日與明細筆數不符：{listed}{more}；請以 MANUAL_MARKET_RETURN_FROM={first}、MANUAL_MARKET_RETURN_TO={last} 重跑 test_backfill_market_forward_returns",
        dates.len()
    )]
}

/// 未來報酬短天期缺漏的基準日：列出最早的幾天與總天數，並提示重跑的起日。
///
/// 回補以「結束日」逐日計算，從最早缺漏的基準日起重跑到今天就會補齊之後各天期。
pub(super) fn forward_return_gap_problems(gaps: &[ForwardReturnGap]) -> Vec<String> {
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

/// ETF 淨值漏抓、正式淨值沒回填與基本資料停更。
///
/// 只看台股交易日：台股休市時海外 ETF 仍有當天的淨值，筆數本來就少。比較基準是檢查期間
/// 之前、交易日的最多列數（新上市只會讓列數變多）；最新交易日的正式淨值要到下一個營業日
/// 才回填，不列入。
pub(super) fn etf_problems(
    snapshot: &DataHealthSnapshot,
    from: NaiveDate,
    today: NaiveDate,
    trading_days: &[NaiveDate],
) -> Vec<String> {
    let rows_on = |date: NaiveDate| {
        snapshot
            .etf_nav_counts
            .iter()
            .find(|count| count.date == date)
    };
    let baseline = trading_days
        .iter()
        .filter(|date| **date < from)
        .filter_map(|date| rows_on(*date).map(|count| count.rows))
        .max();
    let mut problems = Vec::new();
    for date in trading_days
        .iter()
        .rev()
        .filter(|date| (from..today).contains(*date))
    {
        match (rows_on(*date), baseline) {
            (None, _) => problems.push(format!("{date} 沒有 ETF 淨值")),
            (Some(count), Some(baseline)) if count.rows * 100 < baseline * ETF_NAV_MIN_PERCENT => {
                problems.push(format!(
                    "{date} ETF 淨值只有 {} 檔（近期 {baseline} 檔）",
                    count.rows
                ));
            }
            _ => {}
        }
        if let Some(count) = rows_on(*date).filter(|_| Some(date) != trading_days.first())
            && count.missing_nav * 100 > count.rows * ETF_MISSING_NAV_LIMIT_PERCENT
        {
            problems.push(format!(
                "{date} 有 {}／{} 檔 ETF 沒有正式淨值（下一個營業日的回填沒成功）",
                count.missing_nav, count.rows
            ));
        }
    }
    let stale = &snapshot.stale_etf_profiles;
    if !stale.is_empty() {
        let mut listed = stale
            .iter()
            .take(STALE_ETF_PROFILES_LISTED)
            .cloned()
            .collect::<Vec<_>>()
            .join("、");
        if stale.len() > STALE_ETF_PROFILES_LISTED {
            listed.push('…');
        }
        problems.push(format!(
            "{} 檔 ETF 基本資料超過 {ETF_PROFILE_MAX_AGE_DAYS} 天沒更新：{listed}",
            stale.len()
        ));
    }
    problems
}

/// 組成 Telegram 週報。
///
/// 通知管道以 MarkdownV2 發送，週報本身沒有格式標記，因此整則跳脫保留字元；
/// 日期的 `-` 沒跳脫時 Telegram 會整則拒收（400）。
/// 財報落後項目：查詢官方季報公布已久、損益表仍沒有那一季的股票。
pub(super) async fn lagging_check(repo: &impl FinancialReportRepository) -> Check {
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
pub(super) fn lagging_problems(lagging: Option<&[LaggingStatement]>) -> Vec<String> {
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
