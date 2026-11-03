//! 從 Yahoo 補缺漏事件與配股金額的測試。

use super::*;

/// 缺漏事件從 Yahoo 補：挑同一天除權息的明細，取 Yahoo 的所屬年度與期別。
#[test]
fn select_fills_takes_the_yahoo_detail_of_the_same_day() {
    let mut second_h2 = row(
        10,
        2024,
        "H2",
        dec!(0.4863),
        dec!(0.214),
        "2024-09-25",
        "2024-09-25",
    );
    second_h2.year_of_dividend = 2023;
    let unrelated = row(11, 2024, "H1", dec!(0.1), Decimal::ZERO, "2024-03-01", "-");
    let event = cash_event("2024-09-25", dec!(0.4863));

    let chosen = select_fills(&[&event], &[unrelated, second_h2], &[], &HashSet::new());
    assert_eq!(chosen.len(), 1);
    assert_eq!(chosen[0].quarter, "H2");
    assert_eq!(chosen[0].year_of_dividend, 2023);
}

/// 目標鍵上已有一列且已對上交易所事件時不覆蓋；同一筆 Yahoo 明細也不會補兩次。
#[test]
fn select_fills_never_overwrites_a_confirmed_row() {
    let confirmed = row(
        1,
        2024,
        "H2",
        dec!(0.0919),
        Decimal::ZERO,
        "2023-12-12",
        "-",
    );
    let mut yahoo_row = row(10, 2024, "H2", dec!(0.5), Decimal::ZERO, "2024-09-25", "-");
    yahoo_row.year_of_dividend = confirmed.year_of_dividend;
    let event = cash_event("2024-09-25", dec!(0.5));
    let used = HashSet::from([1]);

    assert!(
        select_fills(
            &[&event],
            std::slice::from_ref(&yahoo_row),
            std::slice::from_ref(&confirmed),
            &used
        )
        .is_empty()
    );
    // 那列沒有對上任何交易所事件（是錯的或舊的）時可以覆蓋。
    assert_eq!(
        select_fills(
            &[&event],
            &[yahoo_row.clone()],
            &[confirmed],
            &HashSet::new()
        )
        .len(),
        1
    );
    // 兩個缺漏事件指到同一筆明細時只補一次。
    let twice = select_fills(&[&event, &event], &[yahoo_row], &[], &HashSet::new());
    assert_eq!(twice.len(), 1);
}

/// Yahoo 也找不到同一天的明細時不補。
#[test]
fn select_fills_skips_events_without_a_yahoo_match() {
    let yahoo_row = row(10, 2024, "H2", dec!(0.5), Decimal::ZERO, "2024-09-26", "-");
    let event = cash_event("2024-09-25", dec!(0.5));
    assert!(select_fills(&[&event], &[yahoo_row], &[], &HashSet::new()).is_empty());
}

/// 證交所「權息」對上了只有現金的列：記下來，再用 Yahoo 同一除權日的配股補上（2327 國巨）。
#[test]
fn missing_stock_amount_is_filled_from_yahoo() {
    let rows = existing(vec![row(
        1,
        2024,
        "",
        dec!(2.0),
        Decimal::ZERO,
        "2024-08-15",
        "-",
    )]);
    let event = ExDividendAnnouncement {
        is_stock: true,
        cash_dividend: None,
        ..cash_event("2024-08-15", Decimal::ZERO)
    };
    let plan = build_plan(std::slice::from_ref(&event), &rows, AS_OF);
    assert_eq!(plan.stock_unknown.len(), 1);
    assert_eq!(plan.stock_unknown[0].0, 1);

    let yahoo = row(
        10,
        2024,
        "",
        dec!(2.0),
        dec!(1.9484),
        "2024-08-15",
        "2024-08-15",
    );
    let fixed = stock_from_yahoo(&rows["1109"][0], &event, &[yahoo]).expect("應補上配股");
    assert_eq!(fixed.stock_dividend, dec!(1.9484));
    assert_eq!(fixed.sum, dec!(3.9484));
    assert_eq!(fixed.ex_dividend_date_stock, "2024-08-15");
    assert_eq!(fixed.serial, 1);

    // Yahoo 也沒有配股時不動。
    let cash_only = row(10, 2024, "", dec!(2.0), Decimal::ZERO, "2024-08-15", "-");
    assert!(stock_from_yahoo(&rows["1109"][0], &event, &[cash_only]).is_none());
}

/// 補入列與事件同一天才算補上；Yahoo 沒有同日資料的事件列為補不到
/// （00990B 2026-09-23 被同一輪補上，就不該再列為缺漏）。
#[test]
fn unresolved_after_fill_lists_only_events_without_a_fill() {
    let filled = cash_event("2026-09-23", dec!(0.08));
    let unfilled = cash_event("2026-10-28", dec!(0.08));
    let stock_fill = row(11, 2026, "Q2", Decimal::ZERO, dec!(0.5), "-", "2026-07-15");
    let stock_event = cash_event("2026-07-15", dec!(0.5));
    let fills = vec![
        row(
            10,
            2026,
            "M08",
            dec!(0.08),
            Decimal::ZERO,
            "2026-09-23",
            "-",
        ),
        stock_fill,
    ];

    let labels = |events: Vec<ExDividendAnnouncement>| -> Vec<String> {
        events.iter().map(event_label).collect()
    };
    assert_eq!(
        labels(unresolved_after_fill(
            &[&filled, &unfilled, &stock_event],
            &fills
        )),
        vec!["1109 2026-10-28".to_string()]
    );
    assert_eq!(
        labels(unresolved_after_fill(&[&filled], &[])),
        vec!["1109 2026-09-23".to_string()]
    );
    assert!(unresolved_after_fill(&[], &fills).is_empty());
}

fn yahoo_detail(
    quarter: &str,
    cash: Decimal,
    stock: Decimal,
    ex_cash: &str,
    ex_stock: &str,
) -> crate::infra::crawler::yahoo::dividend::YahooDividendDetail {
    crate::infra::crawler::yahoo::dividend::YahooDividendDetail {
        year: 1991,
        year_of_dividend: 1990,
        quarter: quarter.to_string(),
        cash_dividend: cash,
        stock_dividend: stock,
        ex_dividend_date1: ex_cash.to_string(),
        ex_dividend_date2: ex_stock.to_string(),
        payable_date1: "-".to_string(),
        payable_date2: "-".to_string(),
    }
}

fn event_of(symbol: &str, date: &str, is_stock: bool) -> ExDividendAnnouncement {
    ExDividendAnnouncement {
        stock_symbol: symbol.to_string(),
        is_cash: !is_stock,
        is_stock,
        ..cash_event(date, dec!(1.0))
    }
}

/// 寫入模式：Yahoo 有同日明細的缺漏事件整筆補入、配股缺漏的列補上配股並重算年度合計；
/// Yahoo 沒有的事件與抓不到的股票列為補不到。每檔只抓一次、每檔之後都間隔。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn fill_missing_with_saves_yahoo_rows_and_stock_amounts() {
    const SYMBOL: &str = "79979YF";
    const BROKEN: &str = "79979YX";
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 fill_missing_with_saves_yahoo_rows_and_stock_amounts：無資料庫連接");
        return;
    }
    delete_test_dividends(SYMBOL).await;
    delete_test_dividends(BROKEN).await;

    let repo = PgDividendRepository::new();
    let mut seeded = row(
        0,
        1991,
        "H1",
        dec!(2.0),
        Decimal::ZERO,
        "1991-07-10",
        "1991-07-10",
    );
    seeded.security_code = SYMBOL.to_string();
    repo.save(&seeded).await.expect("種入股利");
    let existing = load_existing(&repo, &[1991]).await.expect("讀取股利");
    let serial = existing[SYMBOL][0].serial;

    let plan = ReconcilePlan {
        missing: vec![
            event_of(SYMBOL, "1991-08-20", false),
            event_of(SYMBOL, "1991-09-30", false),
            event_of(BROKEN, "1991-08-20", false),
        ],
        stock_unknown: vec![(serial, event_of(SYMBOL, "1991-07-10", true))],
        ..ReconcilePlan::default()
    };
    let yahoo = crate::infra::crawler::yahoo::dividend::YahooDividend {
        stock_symbol: SYMBOL.to_string(),
        dividend: vec![(
            1991,
            vec![
                yahoo_detail("H1", dec!(2.0), dec!(0.5), "1991-07-10", "1991-07-10"),
                yahoo_detail("H2", dec!(1.0), Decimal::ZERO, "1991-08-20", "-"),
            ],
        )],
    };
    let mut fetched = Vec::new();
    let mut pauses = 0;

    let fill = fill_missing_with(
        &repo,
        &plan,
        &existing,
        |symbol: String| {
            let result = if symbol == SYMBOL {
                Ok(yahoo.clone())
            } else {
                Err(anyhow::anyhow!("Yahoo 回 404"))
            };
            fetched.push(symbol);
            std::future::ready(result)
        },
        || {
            pauses += 1;
            std::future::ready(())
        },
    )
    .await
    .expect("補缺漏");

    assert_eq!((fill.filled, fill.stock_filled), (2, 1));
    let unresolved: Vec<(String, String)> = fill
        .unresolved
        .iter()
        .map(|event| (event.stock_symbol.clone(), event.ex_date.to_string()))
        .collect();
    assert_eq!(
        unresolved,
        [
            (SYMBOL.to_string(), "1991-09-30".to_string()),
            (BROKEN.to_string(), "1991-08-20".to_string()),
        ]
    );
    assert_eq!(fetched, [SYMBOL, BROKEN]);
    assert_eq!(pauses, 2);

    let reloaded = load_existing(&repo, &[1991]).await.expect("重新讀取");
    let rows = &reloaded[SYMBOL];
    let quarter = |quarter: &str| {
        rows.iter()
            .find(|row| row.quarter == quarter)
            .unwrap_or_else(|| panic!("應有 {quarter} 列：{rows:?}"))
    };
    assert_eq!(
        (quarter("H1").stock_dividend, quarter("H1").sum),
        (dec!(0.5), dec!(2.5))
    );
    assert_eq!(quarter("H2").cash_dividend, dec!(1.0));
    let total = rows
        .iter()
        .find(|row| row.quarter.is_empty() || row.quarter == "A")
        .expect("應重算出年度合計列");
    assert_eq!(
        (total.cash_dividend, total.stock_dividend),
        (dec!(3.0), dec!(0.5))
    );
    assert!(!reloaded.contains_key(BROKEN), "抓不到 Yahoo 的股票不寫入");

    // 正式入口：沒有缺漏事件時不抓 Yahoo、不寫入。
    let empty = fill_missing_from_yahoo(&repo, &ReconcilePlan::default(), &HashMap::new())
        .await
        .expect("空計畫不應失敗");
    assert_eq!((empty.filled, empty.stock_filled), (0, 0));
    assert!(empty.unresolved.is_empty());

    delete_test_dividends(SYMBOL).await;
}
