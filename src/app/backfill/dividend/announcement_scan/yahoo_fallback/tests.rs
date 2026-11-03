use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::super::UNANNOUNCED_DATE;
use super::super::fixtures::{announcement, yahoo_detail};
use super::*;
use crate::infra::crawler::tpex;

/// Yahoo 補救：以除權息日相符找出對應配息，取它的所屬年度、季別與發放年度。
#[test]
fn test_resolve_from_yahoo_matches_by_ex_date() {
    let ann = announcement("6488", (2026, 8, 26), true, false, Some(dec!(10.0)), None);
    let yahoo = YahooDividend {
        stock_symbol: "6488".to_string(),
        dividend: vec![(
            2026,
            vec![
                yahoo_detail(
                    2026,
                    2025,
                    "H1",
                    dec!(5.0),
                    Decimal::ZERO,
                    "2026-03-10",
                    UNANNOUNCED_DATE,
                ),
                yahoo_detail(
                    2026,
                    2025,
                    "H2",
                    dec!(10.0),
                    Decimal::ZERO,
                    "2026-08-26",
                    UNANNOUNCED_DATE,
                ),
            ],
        )],
    };

    let item = resolve_from_yahoo(&ann, &yahoo).expect("ex-date should match");

    assert_eq!(item.quarter, "H2");
    assert_eq!(item.year_of_dividend, 2025);
    assert_eq!(item.paid_year, Some(2026));
    assert_eq!(item.cash_dividend, dec!(10.0));
    // Yahoo 沒有拆分資料，盈餘／公積欄位必須留 0，避免高估盈餘分配率。
    assert_eq!(item.earnings_cash, Decimal::ZERO);
    assert_eq!(item.capital_reserve_cash, Decimal::ZERO);
}

/// 跨年發放的事件被 Yahoo 歸在隔年，只查除權息日那一年會整個漏掉。
#[test]
fn test_resolve_from_yahoo_searches_every_paid_year() {
    let ann = announcement("2882", (2025, 12, 20), true, false, Some(dec!(3.0)), None);
    let yahoo = YahooDividend {
        stock_symbol: "2882".to_string(),
        dividend: vec![(
            2026,
            vec![yahoo_detail(
                2026,
                2025,
                "Q4",
                dec!(3.0),
                Decimal::ZERO,
                "2025-12-20",
                UNANNOUNCED_DATE,
            )],
        )],
    };

    let item = resolve_from_yahoo(&ann, &yahoo).expect("cross-year event should match");
    assert_eq!(item.paid_year, Some(2026));
}

/// 同一年沒有除權息日相符的紀錄時不可硬湊。
#[test]
fn test_resolve_from_yahoo_returns_none_without_match() {
    let ann = announcement("6488", (2026, 8, 26), true, false, Some(dec!(10.0)), None);
    let yahoo = YahooDividend {
        stock_symbol: "6488".to_string(),
        dividend: vec![(
            2026,
            vec![yahoo_detail(
                2026,
                2025,
                "H1",
                dec!(5.0),
                Decimal::ZERO,
                "2026-03-10",
                UNANNOUNCED_DATE,
            )],
        )],
    };

    assert!(resolve_from_yahoo(&ann, &yahoo).is_none());
}

/// 純除權事件要用除權日比對，不能只看除息日。
#[test]
fn test_resolve_from_yahoo_matches_stock_ex_date() {
    let ann = announcement("3234", (2026, 8, 26), false, true, None, Some(dec!(0.1)));
    let yahoo = YahooDividend {
        stock_symbol: "3234".to_string(),
        dividend: vec![(
            2026,
            vec![yahoo_detail(
                2026,
                2025,
                "",
                Decimal::ZERO,
                dec!(1.0),
                UNANNOUNCED_DATE,
                "2026-08-26",
            )],
        )],
    };

    let item = resolve_from_yahoo(&ann, &yahoo).expect("stock ex-date should match");
    assert_eq!(item.stock_dividend, dec!(1.0));
}

/// 對真實的上櫃／ETF 事件實跑第二階段，確認 Yahoo 能補出期別。
///
/// 只查前幾檔以免測試時間過長；同樣不寫入資料庫。
#[tokio::test]
#[ignore]
async fn test_yahoo_fallback_against_live_site() {
    dotenvy::dotenv().ok();

    let otc = tpex::ex_dividend_announcement::visit()
        .await
        .expect("上櫃預告表應可取得");
    let sample: Vec<ExDividendAnnouncement> = otc.into_iter().take(3).collect();

    for announcement in sample {
        let Some(yahoo) = fetch_yahoo_dividend(&announcement.stock_symbol).await else {
            println!("{} 抓取失敗", announcement.stock_symbol);
            continue;
        };

        match resolve_from_yahoo(&announcement, &yahoo) {
            Some(item) => println!(
                "{} {} → 發放年度 {:?} {}{} 現金 {} 股票 {}",
                announcement.stock_symbol,
                announcement.ex_date,
                item.paid_year,
                item.year_of_dividend,
                if item.quarter.is_empty() {
                    "年度".to_string()
                } else {
                    item.quarter.clone()
                },
                item.cash_dividend,
                item.stock_dividend
            ),
            None => println!(
                "{} {} 找不到除權息日相符的 Yahoo 紀錄",
                announcement.stock_symbol, announcement.ex_date
            ),
        }

        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
    }
}

fn unresolved(symbol: &str, ex_date: (i32, u32, u32), reason: UnresolvedReason) -> UnresolvedEvent {
    UnresolvedEvent {
        announcement: announcement(symbol, ex_date, true, false, Some(dec!(10.0)), None),
        reason,
    }
}

fn yahoo_with(symbol: &str, ex_cash: &str) -> YahooDividend {
    YahooDividend {
        stock_symbol: symbol.to_string(),
        dividend: vec![(
            2026,
            vec![yahoo_detail(
                2026,
                2025,
                "H2",
                dec!(10.0),
                Decimal::ZERO,
                ex_cash,
                UNANNOUNCED_DATE,
            )],
        )],
    }
}

/// 逐檔 Yahoo 補救的呼叫紀錄與結果。
struct Run {
    plan: ScanPlan,
    fetched: Vec<String>,
    pauses: usize,
}

/// 以假的 Yahoo 來源跑第二階段：`answers` 指定各代號的回應，沒列到的視為抓取失敗。
async fn run(pending: Vec<UnresolvedEvent>, answers: &[YahooDividend], limit: usize) -> Run {
    let mut plan = ScanPlan {
        unresolved: pending,
        ..ScanPlan::default()
    };
    let mut fetched = Vec::new();
    let mut pauses = 0;
    let sources = YahooSources {
        fetch: |symbol: String| {
            let answer = answers
                .iter()
                .find(|yahoo| yahoo.stock_symbol == symbol)
                .cloned();
            fetched.push(symbol);
            std::future::ready(answer)
        },
        pause: || {
            pauses += 1;
            std::future::ready(())
        },
        limit,
    };
    resolve_with(&mut plan, &HashMap::new(), &HashMap::new(), sources).await;
    Run {
        plan,
        fetched,
        pauses,
    }
}

fn reasons(plan: &ScanPlan) -> Vec<(String, UnresolvedReason)> {
    plan.unresolved
        .iter()
        .map(|event| (event.announcement.stock_symbol.clone(), event.reason))
        .collect()
}

/// 同一檔只查一次、第二檔起才等待；Yahoo 配得到的事件寫入計畫，
/// 配不到、抓不到與不適合問 Yahoo 的事件各帶原因留在待處理清單。
#[tokio::test]
async fn resolve_with_queries_each_symbol_once() {
    let pending = vec![
        unresolved(
            "2881",
            (2026, 8, 26),
            UnresolvedReason::AmbiguousExistingRow,
        ),
        unresolved(
            "6488",
            (2026, 8, 26),
            UnresolvedReason::OverTheCounterUnsupported,
        ),
        unresolved(
            "6488",
            (2026, 9, 1),
            UnresolvedReason::OverTheCounterUnsupported,
        ),
        unresolved("3234", (2026, 8, 26), UnresolvedReason::NoMatchingAllotment),
    ];

    let run = run(pending, &[yahoo_with("6488", "2026-08-26")], 10).await;

    assert_eq!(run.fetched, ["6488", "3234"]);
    assert_eq!(run.pauses, 1);
    assert_eq!(
        run.plan.inserts.len(),
        1,
        "6488 的 08-26 除息由 Yahoo 補出期別"
    );
    assert!(
        run.plan
            .inserts
            .values()
            .all(|dividend| dividend.security_code == "6488" && dividend.quarter == "H2")
    );
    assert_eq!(
        reasons(&run.plan),
        [
            ("2881".to_string(), UnresolvedReason::AmbiguousExistingRow),
            (
                "6488".to_string(),
                UnresolvedReason::NoMatchingYahooDividend
            ),
            ("3234".to_string(), UnresolvedReason::YahooLookupFailed),
        ]
    );
}

/// 查詢檔數到上限後，其餘的事件留待下次排程，不再對 Yahoo 發請求。
#[tokio::test]
async fn resolve_with_stops_at_lookup_limit() {
    let pending = ["1101", "1102", "1103"]
        .iter()
        .map(|symbol| unresolved(symbol, (2026, 8, 26), UnresolvedReason::NoMatchingAllotment))
        .collect();

    let run = run(pending, &[], 2).await;

    assert_eq!(run.fetched, ["1101", "1102"]);
    assert_eq!(run.pauses, 1);
    assert_eq!(
        reasons(&run.plan),
        [
            ("1101".to_string(), UnresolvedReason::YahooLookupFailed),
            ("1102".to_string(), UnresolvedReason::YahooLookupFailed),
            ("1103".to_string(), UnresolvedReason::YahooLookupSkipped),
        ]
    );
}

/// 沒有待處理事件時直接結束，不查 Yahoo。
#[tokio::test]
async fn resolve_with_skips_when_nothing_pending() {
    let run = run(Vec::new(), &[], 10).await;

    assert!(run.fetched.is_empty());
    assert_eq!(run.pauses, 0);
    assert!(run.plan.unresolved.is_empty());

    let mut plan = ScanPlan::default();
    resolve_with_yahoo(&mut plan, &HashMap::new(), &HashMap::new()).await;
    assert!(plan.unresolved.is_empty() && plan.inserts.is_empty());
}
