//! 寫入流程：單日入庫、逐日回補、董監持股與主力進出，以記錄寫入的倉儲替身驗證。

use super::*;

/// 在這個執行緒裝上 tracing subscriber，讓 `tracing::info!` 真的格式化參數（沒有 subscriber 時參數不會求值）。
fn log_to_test_output() -> tracing::subscriber::DefaultGuard {
    tracing::subscriber::set_default(tracing_subscriber::fmt().with_test_writer().finish())
}

fn one_flow(symbol: &str) -> Result<HashMap<String, InstitutionalFlow>> {
    Ok(HashMap::from([(
        symbol.to_string(),
        InstitutionalFlow {
            foreign: 1,
            trust: 2,
            dealer: 3,
        },
    )]))
}

fn one_margin() -> Result<HashMap<String, MarginBalance>> {
    Ok(HashMap::from([(
        "2330".to_string(),
        MarginBalance {
            margin_previous: 1,
            margin_today: 2,
            short_previous: 3,
            short_today: 4,
        },
    )]))
}

/// 單日入庫：先寫三大法人、再寫融資融券，最後只重算當天的連續天數；寫入失敗回錯誤。
#[tokio::test]
async fn write_daily_saves_both_then_refreshes_that_day() {
    use crate::domain::chip::test_double::RecordingChipRepository;

    let date = ymd(2026, 10, 12);
    let (summary, flows, margins) = collect_daily(
        date,
        [one_flow("2330"), one_flow("6488")],
        [one_margin(), Err(anyhow::anyhow!("逾時"))],
    )
    .expect("部分來源失敗仍可整理");
    let repo = RecordingChipRepository::default();
    let summary = write_daily(&repo, date, summary, &flows, &margins)
        .await
        .expect("寫入成功");
    assert_eq!(
        repo.calls(),
        [
            "institutional 2026-10-12 2",
            "margin 2026-10-12 1",
            "streaks 2026-10-12 2026-10-12"
        ]
    );
    assert_eq!(
        summary,
        DailySummary {
            institutional: 2,
            margin: 1,
            failed_sources: 1,
            rows_written: 3,
            streaks_updated: 5
        }
    );

    let broken = RecordingChipRepository::failing();
    assert!(
        write_daily(&broken, date, DailySummary::default(), &flows, &margins)
            .await
            .is_err(),
        "寫入失敗要回錯誤"
    );
    assert_eq!(broken.calls(), ["institutional 2026-10-12 2"]);
}

/// 逐日回補：單日失敗只略過，每天之後都停頓，最後從第一天重算連續天數到今天。
#[tokio::test]
async fn backfill_dates_skips_failed_days_and_refreshes_from_first_day() {
    let _log = log_to_test_output();
    use crate::domain::chip::test_double::RecordingChipRepository;

    let repo = RecordingChipRepository::default();
    let mut saved = Vec::new();
    let mut pauses = 0;
    let written = backfill_dates(
        &repo,
        &[ymd(2026, 10, 1), ymd(2026, 10, 2)],
        ymd(2026, 10, 15),
        |date| {
            saved.push(date);
            let result = if date == ymd(2026, 10, 1) {
                Ok(DailySummary {
                    rows_written: 4,
                    ..DailySummary::default()
                })
            } else {
                Err(anyhow::anyhow!("休市"))
            };
            async move { result }
        },
        || {
            pauses += 1;
            async {}
        },
    )
    .await
    .expect("單日失敗不中止");
    assert_eq!(saved, [ymd(2026, 10, 1), ymd(2026, 10, 2)]);
    assert_eq!(pauses, 2);
    assert_eq!(written, 4 + 5, "寫入列數加上連續天數更新列數");
    assert_eq!(repo.calls(), ["streaks 2026-10-01 2026-10-15"]);

    let empty = RecordingChipRepository::default();
    let written = backfill_dates(
        &empty,
        &[],
        ymd(2026, 10, 15),
        |_| async { Ok(DailySummary::default()) },
        || async {},
    )
    .await
    .expect("沒有交易日");
    assert_eq!(written, 0);
    assert!(empty.calls().is_empty(), "沒有交易日就不重算");
}

/// 董監持股：一邊失敗只寫另一邊，兩邊都失敗回錯誤且不寫入。
#[tokio::test]
async fn save_insider_holdings_writes_what_was_fetched() {
    let _log = log_to_test_output();
    use crate::domain::chip::test_double::RecordingChipRepository;

    let holding = |symbol: &str| InsiderHolding {
        stock_symbol: symbol.to_string(),
        month: ymd(2026, 9, 1),
        title: "董事長".to_string(),
        name: "測試".to_string(),
        shares: 1_000,
        pledged: 0,
        related_pledged: 0,
    };
    let repo = RecordingChipRepository::default();
    save_insider_holdings(
        &repo,
        [
            Ok(vec![holding("2330"), holding("2317")]),
            Err(anyhow::anyhow!("逾時")),
        ],
    )
    .await
    .expect("一邊失敗不算錯誤");
    assert_eq!(repo.calls(), ["insider 2"]);

    let untouched = RecordingChipRepository::default();
    let err = save_insider_holdings(
        &untouched,
        [Err(anyhow::anyhow!("封鎖")), Err(anyhow::anyhow!("封鎖"))],
    )
    .await
    .expect_err("兩邊都失敗");
    assert!(err.to_string().contains("都抓取失敗"), "{err}");
    assert!(untouched.calls().is_empty());
}

/// 主力進出入庫：寫入失敗只記錄，不往外拋。
#[tokio::test]
async fn save_broker_flows_to_writes_and_swallows_failures() {
    let _log = log_to_test_output();
    use crate::domain::chip::test_double::RecordingChipRepository;

    let flow = BrokerFlow {
        stock_symbol: "2884".to_string(),
        date: ymd(2026, 10, 5),
        buyers: Vec::new(),
        sellers: Vec::new(),
        buy_total: 0,
        sell_total: 0,
    };
    let repo = RecordingChipRepository::default();
    save_broker_flows_to(&repo, std::slice::from_ref(&flow)).await;
    assert_eq!(repo.calls(), ["broker_flows 1"]);

    let broken = RecordingChipRepository::failing();
    save_broker_flows_to(&broken, &[flow]).await;
    assert_eq!(broken.calls(), ["broker_flows 1"]);
}

/// 回補區間內沒有交易日：不抓、不重算，回 0（臨時庫）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn backfill_daily_without_trading_days_writes_nothing() {
    dotenvy::dotenv().ok();
    if crate::infra::database::ping().await.is_err() {
        println!("跳過 backfill_daily_without_trading_days_writes_nothing：無資料庫連接");
        return;
    }
    let written = backfill_daily(ymd(1980, 1, 1), ymd(1980, 1, 31))
        .await
        .expect("沒有交易日不是錯誤");
    assert_eq!(written, 0);
}

/// 沒有主力進出資料時照常結束（臨時庫）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn save_broker_flows_accepts_empty_input() {
    dotenvy::dotenv().ok();
    if crate::infra::database::ping().await.is_err() {
        println!("跳過 save_broker_flows_accepts_empty_input：無資料庫連接");
        return;
    }
    save_broker_flows(&[]).await;
}
