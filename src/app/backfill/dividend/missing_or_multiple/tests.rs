//! 股利補抓（季配／半年配與缺漏）的單元與整合測試。

use chrono::Local;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::plan::*;
use super::*;
use crate::app::backfill::acl::YahooDividendAclMapper;
use crate::infra::cache::SHARE;

#[test]
fn test_should_process_dividend_year() {
    let target_year = 2025;
    assert!(should_process_dividend_year(target_year, 2025));
    assert!(should_process_dividend_year(target_year, 2024));
    assert!(!should_process_dividend_year(target_year, 2023));
    assert!(!should_process_dividend_year(target_year, 2026));
}

#[test]
fn refresh_skip_ttl_stays_within_two_to_four_days() {
    for _ in 0..1_000 {
        let ttl = refresh_skip_ttl_seconds();
        assert!((REFRESH_SKIP_TTL_MIN_SECONDS..=REFRESH_SKIP_TTL_MAX_SECONDS).contains(&ttl));
    }
}

#[test]
fn refresh_run_defers_after_budget_is_used_up() {
    let mut run = RefreshRun::new(MAX_FETCHES_PER_RUN + 150, MAX_FETCHES_PER_RUN);
    // 其中 100 檔已有跳過旗標，不會呼叫 try_fetch。
    let to_fetch = MAX_FETCHES_PER_RUN + 50;
    let accepted = (0..to_fetch).filter(|_| run.try_fetch()).count();

    assert_eq!(accepted, MAX_FETCHES_PER_RUN);
    assert_eq!(run.fetched, MAX_FETCHES_PER_RUN);
    assert_eq!(run.deferred, to_fetch - MAX_FETCHES_PER_RUN);
    assert_eq!(run.skipped(), 100);
    assert_eq!(
        run.to_string(),
        format!(
            "candidates={} fetched={} skipped=100 deferred={} unvisited=0",
            MAX_FETCHES_PER_RUN + 150,
            MAX_FETCHES_PER_RUN,
            to_fetch - MAX_FETCHES_PER_RUN
        )
    );
}

#[test]
fn refresh_run_within_budget_defers_nothing() {
    let mut run = RefreshRun::new(10, MAX_FETCHES_PER_RUN);
    assert!((0..4).all(|_| run.try_fetch()));
    assert_eq!(
        run.to_string(),
        "candidates=10 fetched=4 skipped=6 deferred=0 unvisited=0"
    );
}

#[test]
fn test_make_dividend_key() {
    assert_eq!(make_dividend_key("2454", 2024, "Q4"), "2454-2024-Q4");
}

/// 建立測試用的股利實體；只有比對會用到的欄位需要指定。
fn dividend_for_diff(cash: Decimal, stock: Decimal, ex_dividend_date_stock: &str) -> Dividend {
    let now = Local::now();
    Dividend {
        serial: 0,
        year: 2026,
        year_of_dividend: 2025,
        quarter: "H2".to_string(),
        security_code: "2753".to_string(),
        earnings_cash_dividend: Decimal::ZERO,
        capital_reserve_cash_dividend: Decimal::ZERO,
        cash_dividend: cash,
        earnings_stock_dividend: Decimal::ZERO,
        capital_reserve_stock_dividend: Decimal::ZERO,
        stock_dividend: stock,
        sum: cash + stock,
        payout_ratio_cash: Decimal::ZERO,
        payout_ratio_stock: Decimal::ZERO,
        payout_ratio: Decimal::ZERO,
        ex_dividend_date_cash: "2026-07-02".to_string(),
        ex_dividend_date_stock: ex_dividend_date_stock.to_string(),
        payable_date_cash: "2026-07-30".to_string(),
        payable_date_stock: "-".to_string(),
        created_time: now,
        updated_time: now,
    }
}

/// 2753 的實況：資料庫只有現金 7.5，來源已補上併入 25H2 的 0.5 元配股與除權日，
/// 必須判定為有差異才寫得回去。
#[test]
fn dividend_content_differs_detects_merged_stock_dividend() {
    let existing = dividend_for_diff(dec!(7.5), Decimal::ZERO, "-");
    let incoming = dividend_for_diff(dec!(7.5), dec!(0.5), "2026-08-26");

    assert!(dividend_content_differs(&existing, &incoming));
}

/// 盈餘分配率由 Goodinfo 另外回補，Yahoo 一律送 0；只有分配率不同不算內容差異，
/// 否則每次採集都會把已回補的分配率再洗掉一次。
#[test]
fn dividend_content_differs_ignores_payout_ratio() {
    let existing = dividend_for_diff(dec!(7.5), dec!(0.5), "2026-08-26");
    let mut incoming = existing.clone();
    incoming.payout_ratio = dec!(114);
    incoming.payout_ratio_cash = dec!(114);

    assert!(!dividend_content_differs(&existing, &incoming));
}

fn yahoo_detail(
    year: i32,
    year_of_dividend: i32,
    quarter: &str,
    cash: Decimal,
) -> yahoo::dividend::YahooDividendDetail {
    yahoo::dividend::YahooDividendDetail {
        year,
        year_of_dividend,
        quarter: quarter.to_owned(),
        cash_dividend: cash,
        stock_dividend: Decimal::ZERO,
        ex_dividend_date1: "2026-03-17".to_owned(),
        ex_dividend_date2: "-".to_owned(),
        payable_date1: "2026-04-09".to_owned(),
        payable_date2: "-".to_owned(),
    }
}

/// 寫入計畫：只處理今年與去年的股利所屬年度；快取中內容一致的不寫、有更正的要寫；
/// 有分期明細的發放年度一律重算合計（即使明細都沒變），年配列不觸發重算。
#[test]
fn plan_stock_dividends_filters_and_detects_changes() {
    let mut yahoo_dividend = yahoo::dividend::YahooDividend::new("2330".to_owned());
    yahoo_dividend.dividend = vec![
        (
            2026,
            vec![
                yahoo_detail(2026, 2025, "Q4", dec!(6)),
                yahoo_detail(2026, 2025, "Q3", dec!(6)),
                yahoo_detail(2026, 2026, "Q1", dec!(7)),
            ],
        ),
        (2025, vec![yahoo_detail(2025, 2024, "Q4", dec!(5))]),
        (2024, vec![yahoo_detail(2024, 2023, "", dec!(11))]),
    ];

    // Q4 與來源一致 → 不寫；Q3 金額被更正 → 要寫。
    let unchanged = YahooDividendAclMapper::from_command(&YahooDividendAclMapper::from_dto(
        "2330",
        &yahoo_detail(2026, 2025, "Q4", dec!(6)),
    ));
    let mut outdated = unchanged.clone();
    outdated.quarter = "Q3".to_owned();
    outdated.cash_dividend = dec!(5.5);
    let cache: HashMap<String, Dividend> = [
        (make_dividend_key("2330", 2025, "Q4"), unchanged),
        (make_dividend_key("2330", 2025, "Q3"), outdated),
    ]
    .into_iter()
    .collect();

    let plan = plan_stock_dividends(2026, "2330", &yahoo_dividend, &cache);
    let saved: Vec<(i32, &str)> = plan
        .to_save
        .iter()
        .map(|d| (d.year_of_dividend, d.quarter.as_str()))
        .collect();
    // 2024 年度屬於前前年、2023 年配更早，都不處理。
    assert_eq!(saved, vec![(2025, "Q3"), (2026, "Q1")]);
    assert_eq!(
        plan.refresh_years.into_iter().collect::<Vec<_>>(),
        vec![2026]
    );

    let annual_only = yahoo::dividend::YahooDividend {
        stock_symbol: "2887".to_owned(),
        dividend: vec![(2026, vec![yahoo_detail(2026, 2025, "", dec!(1))])],
    };
    let plan = plan_stock_dividends(2026, "2887", &annual_only, &HashMap::new());
    assert_eq!(plan.to_save.len(), 1);
    assert!(plan.refresh_years.is_empty(), "年配列不需重算合計");
}

#[test]
fn test_yahoo_dividend_to_entity_maps_fields_and_sum() {
    let d = yahoo::dividend::YahooDividendDetail {
        year: 2025,
        year_of_dividend: 2024,
        quarter: "Q4".to_string(),
        cash_dividend: dec!(3.5),
        stock_dividend: dec!(0.2),
        ex_dividend_date1: "2025-07-01".to_string(),
        ex_dividend_date2: "2025-07-02".to_string(),
        payable_date1: "2025-08-01".to_string(),
        payable_date2: "2025-08-02".to_string(),
    };

    let cmd = YahooDividendAclMapper::from_dto("2454", &d);
    let e = YahooDividendAclMapper::from_command(&cmd);
    assert_eq!(e.security_code, "2454");
    assert_eq!(e.year, 2025);
    assert_eq!(e.year_of_dividend, 2024);
    assert_eq!(e.quarter, "Q4");
    assert_eq!(e.cash_dividend, dec!(3.5));
    assert_eq!(e.stock_dividend, dec!(0.2));
    assert_eq!(e.sum, dec!(3.7));
    assert_eq!(e.ex_dividend_date_cash, "2025-07-01");
    assert_eq!(e.ex_dividend_date_stock, "2025-07-02");
    assert_eq!(e.payable_date_cash, "2025-08-01");
    assert_eq!(e.payable_date_stock, "2025-08-02");
}

#[tokio::test]
#[ignore]
async fn test_backfill_missing_or_multiple_dividends_live() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    backfill_missing_or_multiple_dividends(2025)
        .await
        .expect("backfill_missing_or_multiple_dividends failed");
}

#[tokio::test]
#[ignore]
async fn test_backfill_recent_dividends_for_stock_live() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let year = 2026;
    let dividend_repo = PgDividendRepository::new();
    let multiple_dividends = dividend_repo
        .fetch_multiple_dividends_for_year(year)
        .await
        .unwrap();
    let mut multiple_dividend_cache: HashMap<String, Dividend> = HashMap::new();
    for dividend in multiple_dividends {
        multiple_dividend_cache.insert(dividend.key(), dividend);
    }

    let _ = backfill_recent_dividends_for_stock(year, "6123", &multiple_dividend_cache).await;
}

/// 假代號：DV1 已有旗標、DV2 補抓成功、DV3 Yahoo 404、DV4 補抓失敗、DV5 超過額度延後。
const REFRESH_SYMBOLS: [&str; 5] = ["79979DV1", "79979DV2", "79979DV3", "79979DV4", "79979DV5"];

async fn clear_refresh_flags() {
    for symbol in REFRESH_SYMBOLS {
        let _ = CLIENT.delete(&make_cache_key(symbol)).await;
    }
}

fn refresh_symbols() -> Vec<String> {
    REFRESH_SYMBOLS.iter().map(|s| (*s).to_owned()).collect()
}

/// 旗標略過、成功、404、失敗都會寫旗標並計入額度，額度用完的延後且不寫旗標。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn refresh_stocks_skips_flagged_and_defers_past_the_limit() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 refresh_stocks_skips_flagged_and_defers_past_the_limit：無 Redis 連線");
        return;
    }
    clear_refresh_flags().await;
    CLIENT
        .set(make_cache_key("79979DV1"), true, 60)
        .await
        .expect("預先寫入略過旗標");

    let symbols = refresh_symbols();
    let calls = std::sync::Mutex::new(Vec::new());
    let pauses = std::sync::atomic::AtomicUsize::new(0);
    let run = refresh_stocks(
        &symbols,
        3,
        |symbol: &str| {
            calls.lock().expect("calls").push(symbol.to_owned());
            let result = match symbol {
                "79979DV3" => Err(anyhow::Error::new(
                    yahoo::dividend::YahooPageNotFoundError {
                        stock_symbol: symbol.to_owned(),
                        url: "https://example.invalid".to_owned(),
                    },
                )),
                "79979DV4" => Err(anyhow::anyhow!("Yahoo 測試失敗")),
                _ => Ok(()),
            };
            async move { result }
        },
        || {
            pauses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            async {}
        },
        || false,
    )
    .await
    .expect("refresh_stocks");

    let mut flags = Vec::new();
    for symbol in REFRESH_SYMBOLS {
        flags.push(
            CLIENT
                .get_bool(&make_cache_key(symbol))
                .await
                .unwrap_or(false),
        );
    }
    clear_refresh_flags().await;

    assert_eq!(
        *calls.lock().expect("calls"),
        ["79979DV2", "79979DV3", "79979DV4"]
    );
    assert_eq!(pauses.load(std::sync::atomic::Ordering::Relaxed), 3);
    assert_eq!(
        run.to_string(),
        "candidates=5 fetched=3 skipped=1 deferred=1 unvisited=0"
    );
    assert_eq!(
        flags,
        [true, true, true, true, false],
        "延後的 DV5 不寫旗標"
    );
}

/// 收到關機請求就停在兩檔之間，還沒輪到的不查、不寫旗標。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn refresh_stocks_stops_between_stocks_when_shutdown_is_requested() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!(
            "跳過 refresh_stocks_stops_between_stocks_when_shutdown_is_requested：無 Redis 連線"
        );
        return;
    }
    clear_refresh_flags().await;

    let symbols = refresh_symbols();
    let shutdown = crate::core::shutdown::ShutdownRequest::new();
    let run = refresh_stocks(
        &symbols,
        MAX_FETCHES_PER_RUN,
        |_symbol: &str| {
            // 第一檔之後的間隔收到關機請求，之後不應再補抓。
            if shutdown.is_requested() {
                panic!("收到關機請求後不應再處理下一檔");
            }
            async { Ok(()) }
        },
        || {
            shutdown.request();
            async {}
        },
        || shutdown.is_requested(),
    )
    .await
    .expect("refresh_stocks");

    let second_flag = CLIENT
        .get_bool(&make_cache_key("79979DV2"))
        .await
        .unwrap_or(false);
    clear_refresh_flags().await;

    assert_eq!(
        run.to_string(),
        "candidates=5 fetched=1 skipped=0 deferred=0 unvisited=4"
    );
    assert!(!second_flag, "還沒輪到的股票不寫旗標");
}

#[test]
fn request_jitter_stays_within_one_and_a_half_to_three_seconds() {
    for _ in 0..1_000 {
        let jitter = request_jitter();
        assert!((Duration::from_millis(1500)..=Duration::from_millis(3000)).contains(&jitter));
    }
}

/// 整輪流程：讀候選與多次配息快取後，收到關機請求就在第一檔之前停下，不查 Yahoo。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn refresh_year_reads_candidates_then_stops_when_shutdown_is_requested() {
    dotenvy::dotenv().ok();
    if sqlx::query("SELECT 1")
        .execute(crate::infra::database::get_connection())
        .await
        .is_err()
    {
        println!(
            "跳過 refresh_year_reads_candidates_then_stops_when_shutdown_is_requested：無資料庫連接"
        );
        return;
    }

    refresh_year(2099, || true).await.expect("提早結束不是錯誤");
}
