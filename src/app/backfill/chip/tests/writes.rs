//! 寫入流程：單日入庫、逐日回補、董監持股與主力進出，以記錄寫入的倉儲替身驗證。

use super::*;

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

/// 單日入庫：先寫三大法人、再寫融資融券，最後只重算當天的連續天數。
#[tokio::test]
async fn write_daily_saves_both_then_refreshes_that_day() {
    use crate::domain::chip::test_double::RecordingChipRepository;

    let date = ymd(2026, 10, 12);
    let repo = RecordingChipRepository::default();
    let summary = write_daily(
        &repo,
        date,
        [one_flow("2330"), one_flow("6488")],
        [one_margin(), Err(anyhow::anyhow!("逾時"))],
    )
    .await
    .expect("部分來源失敗仍寫入");
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

    let untouched = RecordingChipRepository::default();
    fn failed<T>() -> Result<T> {
        Err(anyhow::anyhow!("封鎖"))
    }
    assert!(
        write_daily(&untouched, date, [failed(), failed()], [failed(), failed()])
            .await
            .is_err()
    );
    assert!(untouched.calls().is_empty(), "四個來源都失敗時不寫入");

    let broken = RecordingChipRepository::failing();
    assert!(
        write_daily(
            &broken,
            date,
            [one_flow("2330"), one_flow("6488")],
            [one_margin(), one_margin()]
        )
        .await
        .is_err(),
        "寫入失敗要回錯誤"
    );
    assert_eq!(broken.calls(), ["institutional 2026-10-12 2"]);
}

/// 逐日回補：單日失敗只略過，每天之後都停頓，最後從第一天重算連續天數到今天。
#[tokio::test]
async fn backfill_dates_skips_failed_days_and_refreshes_from_first_day() {
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
