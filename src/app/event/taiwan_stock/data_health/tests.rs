//! 資料健康週報的測試：落後天數判斷、衍生表與未來報酬缺漏、訊息格式。

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
            ("daily_market_valuation", day(9)),
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
        message.starts_with("資料健康週報 2026\\-10\\-04～2026\\-10\\-10\n全部 10 項正常"),
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
    snapshot.chip_streak_missing = 1_890;

    let checks = evaluate(&snapshot, day(4), day(10), Some(&[]));
    let message = report(day(4), day(10), &checks);

    assert!(message.contains("3 項需要處理"), "{message}");
    assert!(
        message.contains("⚠️ 籌碼連續天數\n  • 1890 列連續買賣超天數未計算"),
        "{message}"
    );
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

fn lagging(symbol: &str, published: u32) -> LaggingStatement {
    LaggingStatement {
        stock_symbol: symbol.to_owned(),
        fiscal_year: 2026,
        quarter: "Q2".to_owned(),
        published_on: day(published),
    }
}

/// 財報落後：沒有落後不列、列出代號與期別與公布日、超過上限只計數；查詢失敗也是問題。
#[test]
fn lagging_statements_are_listed_with_their_period() {
    assert!(lagging_problems(Some(&[])).is_empty());
    assert_eq!(
        lagging_problems(Some(&[lagging("3169", 1), lagging("6274", 2)])),
        [
            "2 檔官方季報已公布 7 天以上、損益表還沒有那一季：3169 2026Q2（10-01 公布）、6274 2026Q2（10-02 公布）"
        ]
    );
    let many: Vec<LaggingStatement> = (0..12)
        .map(|i| lagging(&format!("{}", 1101 + i), 1))
        .collect();
    let problems = lagging_problems(Some(&many));
    assert!(problems[0].starts_with("12 檔"), "{problems:?}");
    assert!(
        problems[0].ends_with("1110 2026Q2（10-01 公布）、…共 12 檔"),
        "{problems:?}"
    );
    assert_eq!(lagging_problems(None), ["查詢失敗，見 warn 日誌"]);
}

/// 回傳固定落後名單（`None` 時查詢失敗）的假倉儲。
struct FakeReports(Option<Vec<LaggingStatement>>);

#[async_trait::async_trait]
impl FinancialReportRepository for FakeReports {
    async fn save_income_statements(
        &self,
        _: &[crate::domain::financial::statement::IncomeStatement],
    ) -> Result<u64> {
        unimplemented!("健康檢查不寫入")
    }

    async fn save_balance_sheets(
        &self,
        _: &[crate::domain::financial::statement::BalanceSheet],
    ) -> Result<u64> {
        unimplemented!("健康檢查不寫入")
    }

    async fn save_cash_flow_statements(
        &self,
        _: &[crate::domain::financial::statement::CashFlowStatement],
    ) -> Result<u64> {
        unimplemented!("健康檢查不寫入")
    }

    async fn fetch_lagging_statements(
        &self,
        published_within_days: i32,
        min_age_days: i32,
    ) -> Result<Vec<LaggingStatement>> {
        assert_eq!(
            (published_within_days, min_age_days),
            (LAGGING_LOOKBACK_DAYS, LAGGING_MIN_AGE_DAYS)
        );
        self.0.clone().ok_or_else(|| anyhow::anyhow!("db down"))
    }
}

/// 財報落後項目以一年內、公布滿 7 天查詢；查詢失敗列為問題。
#[tokio::test]
async fn lagging_check_queries_stale_reports() {
    let check = lagging_check(&FakeReports(Some(vec![lagging("3169", 1)]))).await;
    assert_eq!(check.name, "財報落後");
    assert_eq!(check.problems.len(), 1);
    assert!(
        lagging_check(&FakeReports(Some(Vec::new())))
            .await
            .problems
            .is_empty()
    );
    assert_eq!(
        lagging_check(&FakeReports(None)).await.problems,
        ["查詢失敗，見 warn 日誌"]
    );
}
