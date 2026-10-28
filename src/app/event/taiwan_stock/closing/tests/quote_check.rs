//! 收盤行情完整性的測試：成交檔數短缺、臨時停市判斷、重跑與告警內容。

use super::*;

/// 上櫃整天沒抓到（2026-03-17 只剩零量補值列）時判定不完整；上市正常不列入。
#[test]
fn shortfalls_flag_a_missing_market() {
    let counts = vec![
        count(day(13), 2, 1_010),
        count(day(13), 4, 880),
        count(day(16), 2, 1_005),
        count(day(16), 4, 875),
        count(day(17), 2, 1_002),
    ];
    assert_eq!(
        shortfalls(day(17), &counts),
        vec![Shortfall {
            market: "上櫃",
            traded: 0,
            baseline: 880,
        }]
    );
}

/// 冷門股沒成交造成的小幅波動不算不完整；低於近期最大值一半才算。
#[test]
fn shortfalls_tolerate_normal_variation() {
    let counts = vec![
        count(day(16), 2, 1_000),
        count(day(16), 4, 880),
        count(day(17), 2, 950),
        count(day(17), 4, 440),
    ];
    assert!(shortfalls(day(17), &counts).is_empty());

    let partial = vec![
        count(day(16), 4, 880),
        count(day(17), 2, 1_000),
        count(day(17), 4, 439),
    ];
    assert_eq!(shortfalls(day(17), &partial)[0].traded, 439);
}

/// 沒有近期資料可比較時，只要當天有成交就算完整；兩個市場都 0 檔則都列出。
#[test]
fn shortfalls_without_baseline() {
    assert!(shortfalls(day(17), &[count(day(17), 2, 10), count(day(17), 4, 5)]).is_empty());
    assert_eq!(shortfalls(day(17), &[]).len(), 2);
}

/// 臨時停市：上市、上櫃都 0 檔且沒有加權指數才成立；任一邊有成交或指數已寫入都不算。
#[test]
fn unscheduled_closure_requires_both_markets_and_index_missing() {
    let closed = shortfalls(day(17), &[count(day(16), 2, 1_000), count(day(16), 4, 880)]);
    assert!(is_unscheduled_closure(&closed, 0));
    assert!(!is_unscheduled_closure(&closed, 1));

    let listed_only = shortfalls(
        day(17),
        &[
            count(day(16), 2, 1_000),
            count(day(16), 4, 880),
            count(day(17), 4, 870),
        ],
    );
    assert_eq!(listed_only.len(), 1);
    assert!(!is_unscheduled_closure(&listed_only, 0));

    let partial = shortfalls(
        day(17),
        &[
            count(day(16), 2, 1_000),
            count(day(16), 4, 880),
            count(day(17), 2, 300),
            count(day(17), 4, 200),
        ],
    );
    assert_eq!(partial.len(), 2);
    assert!(!is_unscheduled_closure(&partial, 0));
    assert!(!is_unscheduled_closure(&[], 0));
}

/// 以正式倉儲重現 2026-10-23：前一天兩個市場都有成交，當天沒有行情也沒有加權指數，
/// 重跑（替身）成功後仍是 0 筆 → 判定臨時停市，不告警。假代號與歷史日期，結束時刪除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn ensure_quotes_detects_unscheduled_closure_with_database() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 ensure_quotes_detects_unscheduled_closure_with_database：無資料庫連接");
        return;
    }
    let cleanup = || async {
        for sql in [
            r#"DELETE FROM "DailyQuotes" WHERE stock_symbol IN ('79974', '79975')"#,
            r#"DELETE FROM stocks WHERE stock_symbol IN ('79974', '79975')"#,
        ] {
            let _ = sqlx::query(sql).execute(pool).await;
        }
    };
    cleanup().await;
    let previous = NaiveDate::from_ymd_opt(1999, 3, 16).unwrap();
    let closed = NaiveDate::from_ymd_opt(1999, 3, 17).unwrap();
    for (symbol, market_id) in [("79974", 2), ("79975", 4)] {
        sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id)
               VALUES ($1, $1, $1, $2)"#,
        )
        .bind(symbol)
        .bind(market_id)
        .execute(pool)
        .await
        .expect("插入股票母檔");
        sqlx::query(
            r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "TradingVolume")
               VALUES ($1, $2, 10, 1000)"#,
        )
        .bind(previous)
        .bind(symbol)
        .execute(pool)
        .await
        .expect("插入前一天報價");
    }

    let reran = std::cell::Cell::new(false);
    let check = ensure_quotes(closed, || {
        reran.set(true);
        async { Ok(()) }
    })
    .await;
    cleanup().await;

    assert_eq!(check.expect("檢查不應失敗"), QuoteCheck::UnscheduledClosure);
    assert!(reran.get(), "判定停市前要先重跑一次");
}

/// 執行 [`check_quotes`]：`counts` 依序是重跑前、重跑後的成交檔數，回傳結果、是否重跑、是否查了指數。
async fn run_check_quotes(
    counts: [Vec<MarketTradedCount>; 2],
    rerun: Result<()>,
    index: i64,
) -> (QuoteCheck, bool, bool) {
    use std::cell::Cell;

    let calls = Cell::new(0usize);
    let reran = Cell::new(false);
    let index_queried = Cell::new(false);
    let check = check_quotes(
        day(17),
        || {
            let current = counts[calls.get().min(1)].clone();
            calls.set(calls.get() + 1);
            async move { Ok(current) }
        },
        || {
            reran.set(true);
            async move { rerun }
        },
        || {
            index_queried.set(true);
            async move { Ok(index) }
        },
    )
    .await
    .expect("檢查不應失敗");
    (check, reran.get(), index_queried.get())
}

/// 近期兩個市場都有成交，當天沒有任何成交。
fn closed_day_counts() -> Vec<MarketTradedCount> {
    vec![count(day(16), 2, 1_000), count(day(16), 4, 880)]
}

/// 當天兩個市場都正常成交。
fn full_day_counts() -> Vec<MarketTradedCount> {
    let mut counts = closed_day_counts();
    counts.extend([count(day(17), 2, 1_000), count(day(17), 4, 880)]);
    counts
}

#[tokio::test]
async fn check_quotes_complete_skips_rerun() {
    let (check, reran, index_queried) =
        run_check_quotes([full_day_counts(), full_day_counts()], Ok(()), 1).await;
    assert_eq!(check, QuoteCheck::Complete);
    assert!(!reran);
    assert!(!index_queried);
}

/// 2026-10-23：兩個市場與指數都沒有資料，重跑成功但仍是 0 筆 → 臨時停市，不告警。
#[tokio::test]
async fn check_quotes_detects_unscheduled_closure() {
    let (check, reran, index_queried) =
        run_check_quotes([closed_day_counts(), closed_day_counts()], Ok(()), 0).await;
    assert_eq!(check, QuoteCheck::UnscheduledClosure);
    assert!(reran);
    assert!(index_queried);
}

/// 指數已寫入（交易日），行情卻兩邊都 0 筆：照舊告警。
#[tokio::test]
async fn check_quotes_alerts_when_index_exists() {
    let (check, _, _) =
        run_check_quotes([closed_day_counts(), closed_day_counts()], Ok(()), 1).await;
    let QuoteCheck::Rerun(title, body) = check else {
        panic!("指數已寫入時不可判定停市");
    };
    assert_eq!(title, "收盤資料仍不完整");
    assert!(body.contains("重跑後仍不完整"), "{body}");
}

/// TWSE 晚發布：重跑後補齊，告警「已重抓補齊」。
#[tokio::test]
async fn check_quotes_reports_late_data_filled() {
    let (check, reran, _) =
        run_check_quotes([closed_day_counts(), full_day_counts()], Ok(()), 0).await;
    assert!(reran);
    let QuoteCheck::Rerun(title, body) = check else {
        panic!("重跑補齊後要告警");
    };
    assert_eq!(title, "收盤資料已重抓補齊");
    assert!(body.contains("重跑後兩個市場都已完整"), "{body}");
}

/// 重跑失敗時不判定停市，也不用查指數，告警附上錯誤原因。
#[tokio::test]
async fn check_quotes_rerun_failure_is_never_closure() {
    let (check, _, index_queried) = run_check_quotes(
        [closed_day_counts(), closed_day_counts()],
        Err(anyhow::anyhow!("來源逾時")),
        0,
    )
    .await;
    assert!(!index_queried);
    let QuoteCheck::Rerun(title, body) = check else {
        panic!("重跑失敗要告警");
    };
    assert_eq!(title, "收盤資料仍不完整");
    assert!(body.contains("重跑收盤匯總失敗：來源逾時"), "{body}");
}

/// 告警內容依序說明檢查時的缺口、重跑結果與重跑後狀況。
#[test]
fn report_describes_before_rerun_and_after() {
    let missing = vec![Shortfall {
        market: "上櫃",
        traded: 0,
        baseline: 880,
    }];
    assert_eq!(
        report(day(17), &missing, None, &[]),
        "2026-03-17 檢查時：上櫃 0 檔有成交（近期 880 檔）\n已重跑收盤匯總\n重跑後兩個市場都已完整"
    );
    let text = report(day(17), &missing, Some("上櫃收盤資料 0 筆"), &missing);
    assert!(
        text.contains("重跑收盤匯總失敗：上櫃收盤資料 0 筆"),
        "{text}"
    );
    assert!(
        text.contains("重跑後仍不完整：上櫃 0 檔有成交（近期 880 檔）"),
        "{text}"
    );
}
