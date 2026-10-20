//! 交易所資料補 ETF 配息的測試。

use super::*;
use crate::app::backfill::dividend::ex_right_reconcile::exchange_fill::{
    Window, exchange_fills, fill_etf_from_exchange, unresolved_after_exchange_fill,
};

fn etf_event(symbol: &str, date: &str, cash: Decimal) -> ExDividendAnnouncement {
    ExDividendAnnouncement {
        stock_symbol: symbol.to_string(),
        name: "ETF".to_string(),
        market: StockExchangeMarket::OverTheCounter,
        ..cash_event(date, cash)
    }
}

fn etf_row(
    serial: i64,
    symbol: &str,
    year: i32,
    year_of_dividend: i32,
    quarter: &str,
    cash: Decimal,
    date: &str,
) -> Dividend {
    Dividend {
        security_code: symbol.to_string(),
        year_of_dividend,
        ..row(serial, year, quarter, cash, Decimal::ZERO, date, "-")
    }
}

fn window(start: &str, end: &str) -> Window {
    Window {
        start: NaiveDate::parse_from_str(start, DATE_FORMAT).unwrap(),
        end: NaiveDate::parse_from_str(end, DATE_FORMAT).unwrap(),
    }
}

/// 補入列的 `(代號, 發放年度, 所屬年度, 期別, 除息日)`。
fn keys(fills: &[Dividend]) -> Vec<(String, i32, i32, String, String)> {
    fills
        .iter()
        .map(|fill| {
            (
                fill.security_code.clone(),
                fill.year,
                fill.year_of_dividend,
                fill.quarter.clone(),
                fill.ex_dividend_date_cash.clone(),
            )
        })
        .collect()
}

fn key(
    symbol: &str,
    year: i32,
    year_of_dividend: i32,
    quarter: &str,
    date: &str,
) -> (String, i32, i32, String, String) {
    (
        symbol.to_string(),
        year,
        year_of_dividend,
        quarter.to_string(),
        date.to_string(),
    )
}

/// Yahoo 已下架的季配債券 ETF（00794B）：期別取除息日的前一季，現金與日期照交易所。
#[test]
fn exchange_fills_quarterly_etf_without_rows() {
    let events = vec![
        etf_event("00794B", "2019-08-15", dec!(0.181)),
        etf_event("00794B", "2019-11-15", dec!(0.212)),
        etf_event("00794B", "2020-02-18", dec!(0.323)),
    ];
    let fills = exchange_fills(
        &events,
        &events,
        &HashMap::new(),
        window("2019-01-01", "2026-10-05"),
    );

    assert_eq!(
        keys(&fills),
        vec![
            key("00794B", 2019, 2019, "Q2", "2019-08-15"),
            key("00794B", 2019, 2019, "Q3", "2019-11-15"),
            key("00794B", 2020, 2019, "Q4", "2020-02-18"),
        ]
    );
    assert_eq!(fills[0].cash_dividend, dec!(0.181));
    assert_eq!(fills[0].sum, dec!(0.181));
    assert_eq!(fills[0].ex_dividend_date_stock, "-");
}

/// 月配 ETF 7 月初與月底各除息一次（00950B 2026-07-02、07-31），Yahoo 漏列前者：
/// 前一期 M06 已被 07-31 那列占用，改落在除息當月 M07。
#[test]
fn exchange_fills_monthly_etf_falls_back_to_the_ex_month() {
    let official = vec![
        etf_event("00950B", "2026-06-02", dec!(0.065)),
        etf_event("00950B", "2026-07-02", dec!(0.063)),
        etf_event("00950B", "2026-07-31", dec!(0.063)),
        etf_event("00950B", "2026-09-02", dec!(0.063)),
    ];
    let rows = HashMap::from([(
        "00950B".to_string(),
        vec![
            etf_row(1, "00950B", 2026, 2026, "M05", dec!(0.065), "2026-06-02"),
            etf_row(2, "00950B", 2026, 2026, "M06", dec!(0.063), "2026-07-31"),
            etf_row(3, "00950B", 2026, 2026, "M08", dec!(0.063), "2026-09-02"),
        ],
    )]);
    let fills = exchange_fills(
        &official[1..2],
        &official,
        &rows,
        window("2026-01-01", "2026-10-05"),
    );

    assert_eq!(
        keys(&fills),
        vec![key("00950B", 2026, 2026, "M07", "2026-07-02")]
    );
}

/// 半年配：上半年除息屬前一年 H2、下半年除息屬當年 H1；12 月中以後除息視為隔年發放。
#[test]
fn exchange_fills_semiannual_and_december_payment_year() {
    let semiannual = vec![
        etf_event("00718B", "2019-03-07", dec!(0.5)),
        etf_event("00718B", "2019-09-05", dec!(0.5)),
    ];
    let fills = exchange_fills(
        &semiannual,
        &semiannual,
        &HashMap::new(),
        window("2019-01-01", "2026-10-05"),
    );
    assert_eq!(
        keys(&fills),
        vec![
            key("00718B", 2019, 2018, "H2", "2019-03-07"),
            key("00718B", 2019, 2019, "H1", "2019-09-05"),
        ]
    );

    let monthly = vec![
        etf_event("00939", "2025-11-03", dec!(0.07)),
        etf_event("00939", "2025-12-18", dec!(0.07)),
    ];
    let fills = exchange_fills(
        &monthly[1..],
        &monthly,
        &HashMap::new(),
        window("2025-01-01", "2026-10-05"),
    );
    assert_eq!(
        keys(&fills),
        vec![key("00939", 2026, 2025, "M11", "2025-12-18")]
    );
}

/// 不補：個股、配股、30 天內的事件、頻率無法判斷、附近有日期對不上交易所的資料列、
/// 日期未公布但金額相近的資料列。
#[test]
fn exchange_fills_skips_unsafe_events() {
    let window = window("2026-01-01", "2026-10-05");
    let official = vec![
        etf_event("00950B", "2026-06-02", dec!(0.065)),
        etf_event("00950B", "2026-07-02", dec!(0.063)),
        etf_event("00950B", "2026-09-20", dec!(0.063)),
    ];
    let skip = |events: &[ExDividendAnnouncement], rows: &HashMap<String, Vec<Dividend>>| {
        assert!(
            exchange_fills(events, &official, rows, window).is_empty(),
            "{events:?}"
        );
    };
    let none = HashMap::new();

    skip(&[cash_event("2026-07-02", dec!(1.5))], &none);
    let mut stock = etf_event("00950B", "2026-07-02", dec!(0.063));
    stock.is_stock = true;
    skip(&[stock], &none);
    skip(&[official[2].clone()], &none);
    skip(&[etf_event("00999B", "2026-07-02", dec!(0.063))], &none);

    let stale = HashMap::from([(
        "00950B".to_string(),
        vec![etf_row(
            1,
            "00950B",
            2026,
            2026,
            "M06",
            dec!(0.063),
            "2026-07-10",
        )],
    )]);
    skip(&official[1..2], &stale);

    let unannounced = HashMap::from([(
        "00950B".to_string(),
        vec![etf_row(
            1,
            "00950B",
            2026,
            2026,
            "M06",
            dec!(0.064),
            "尚未公布",
        )],
    )]);
    skip(&official[1..2], &unannounced);
}

/// 期間外的舊日期無從判斷是否對得上交易所，不擋補登；兩個候選期別都被占用時不補。
#[test]
fn exchange_fills_ignores_rows_outside_the_window_and_gives_up_when_occupied() {
    let official = vec![
        etf_event("00950B", "2026-07-02", dec!(0.063)),
        etf_event("00950B", "2026-07-31", dec!(0.063)),
    ];
    let window = window("2026-07-01", "2026-10-05");
    let outside = HashMap::from([(
        "00950B".to_string(),
        vec![etf_row(
            1,
            "00950B",
            2026,
            2026,
            "M05",
            dec!(0.065),
            "2026-06-02",
        )],
    )]);
    assert_eq!(
        keys(&exchange_fills(&official[..1], &official, &outside, window)),
        vec![key("00950B", 2026, 2026, "M06", "2026-07-02")]
    );

    let occupied = HashMap::from([(
        "00950B".to_string(),
        vec![
            etf_row(1, "00950B", 2026, 2026, "M06", dec!(0.063), "2026-07-31"),
            etf_row(2, "00950B", 2026, 2026, "M07", dec!(0.063), "2026-08-31"),
        ],
    )]);
    assert!(exchange_fills(&official[..1], &official, &occupied, window).is_empty());
}

/// 補入後只剩沒有同日資料列的事件。
#[test]
fn unresolved_after_exchange_fill_lists_remaining_events() {
    let events = vec![
        etf_event("00794B", "2019-08-15", dec!(0.181)),
        etf_event("00794B", "2019-11-15", dec!(0.212)),
    ];
    let fills = vec![etf_row(
        0,
        "00794B",
        2019,
        2019,
        "Q2",
        dec!(0.181),
        "2019-08-15",
    )];
    assert_eq!(
        unresolved_after_exchange_fill(&events, &fills),
        vec!["00794B 2019-11-15".to_string()]
    );
}

/// 交易所補登寫入資料庫並重算年度合計；不在股票主檔（或已下市）的代號不補。
/// 測試用假代號，結束時刪除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn fill_etf_from_exchange_saves_rows_and_annual_totals() {
    dotenvy::dotenv().ok();
    let repo = PgDividendRepository::new();
    if repo.fetch_by_years(&[1999]).await.is_err() {
        println!("跳過 fill_etf_from_exchange_saves_rows_and_annual_totals：無資料庫連接");
        return;
    }
    let tracked = "00999T";
    let untracked = "00998T";
    delete_test_dividends(tracked).await;
    delete_test_dividends(untracked).await;
    crate::infra::cache::SHARE
        .stocks
        .write()
        .expect("stocks 快取可寫入")
        .insert(
            tracked.to_string(),
            crate::domain::registry::entity::Stock::reconstitute(
                tracked.to_string(),
                "測試債券ETF".to_string(),
                false,
                Decimal::ZERO,
                Decimal::ZERO,
                Decimal::ZERO,
                Local::now(),
                4,
                24,
                0,
                0,
                Decimal::ZERO,
            ),
        );

    let events = vec![
        etf_event(tracked, "2019-08-15", dec!(0.181)),
        etf_event(tracked, "2019-11-15", dec!(0.212)),
        etf_event(untracked, "2019-08-15", dec!(0.5)),
        etf_event(untracked, "2019-11-15", dec!(0.5)),
    ];
    let fills = fill_etf_from_exchange(
        &repo,
        &events,
        &events,
        &HashMap::new(),
        window("2019-01-01", "2019-12-31"),
    )
    .await
    .expect("交易所補登");

    let saved: Vec<Dividend> = repo
        .fetch_by_years(&[2019])
        .await
        .expect("讀取股利")
        .into_iter()
        .filter(|row| row.security_code == tracked || row.security_code == untracked)
        .collect();

    delete_test_dividends(tracked).await;
    crate::infra::cache::SHARE
        .stocks
        .write()
        .expect("stocks 快取可寫入")
        .remove(tracked);

    assert_eq!(keys(&fills).len(), 2, "只補主檔中的代號");
    assert!(saved.iter().all(|row| row.security_code == tracked));
    let mut quarters: Vec<(String, Decimal)> = saved
        .iter()
        .map(|row| (row.quarter.clone(), row.cash_dividend))
        .collect();
    quarters.sort();
    assert_eq!(
        quarters,
        vec![
            (String::new(), dec!(0.393)),
            ("Q2".to_string(), dec!(0.181)),
            ("Q3".to_string(), dec!(0.212)),
        ]
    );
}
