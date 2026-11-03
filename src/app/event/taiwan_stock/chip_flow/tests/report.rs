//! 依當天籌碼挑出要通知的持股（需要 Redis 存連續天數）。

use super::*;

/// 測試用的假代號：A 外資大量、B 沒有明顯動作、C 融資大增、D 當天沒有籌碼資料。
const SYMBOLS: [&str; 4] = ["79976A", "79976B", "79976C", "79976D"];

fn flows() -> HashMap<String, InstitutionalFlow> {
    HashMap::from([
        (SYMBOLS[0].to_owned(), flow(3_000_000, 0, 0)),
        (SYMBOLS[1].to_owned(), flow(10_000, -10_000, 0)),
    ])
}

fn margins() -> HashMap<String, MarginBalance> {
    HashMap::from([(
        SYMBOLS[2].to_owned(),
        MarginBalance {
            margin_previous: 1_000,
            margin_today: 2_000,
            short_previous: 0,
            short_today: 0,
        },
    )])
}

async fn clear_streaks() {
    for symbol in SYMBOLS {
        let _ = CLIENT.delete(&streak_key(symbol)).await;
    }
}

fn reported(reports: &[ChipReport]) -> Vec<&str> {
    reports
        .iter()
        .map(|report| report.stock_symbol.as_str())
        .collect()
}

/// 只挑出符合條件的持股，連續天數寫入 Redis；同一天重跑不重複累加。沒有成交量時比例條件不成立。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn collect_reports_picks_holdings_with_notable_chips() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 collect_reports_picks_holdings_with_notable_chips：無 Redis 連線");
        return;
    }
    clear_streaks().await;
    let symbols: BTreeSet<String> = SYMBOLS.iter().map(|s| (*s).to_owned()).collect();
    let volumes = HashMap::from([
        (SYMBOLS[0].to_owned(), 5_000_000),
        (SYMBOLS[1].to_owned(), 5_000_000),
    ]);

    let (summary, reports) =
        collect_reports(day(6), &symbols, &flows(), &margins(), &volumes).await;
    assert_eq!(
        summary,
        RunSummary {
            holdings: 4,
            with_flow: 2,
            with_margin: 1,
            reported: 2,
        }
    );
    assert_eq!(reported(&reports), ["79976A", "79976C"]);
    assert_eq!(reports[0].volume, Some(5_000_000));
    let saved = Streak {
        date: day(6),
        foreign: 1,
        trust: 0,
    };
    assert_eq!(load_streak(SYMBOLS[0]).await, Some(saved));
    assert_eq!(
        load_streak(SYMBOLS[1]).await,
        Some(Streak {
            date: day(6),
            foreign: 1,
            trust: -1,
        })
    );

    // 同一天重跑：連續天數維持原值。
    collect_reports(day(6), &symbols, &flows(), &margins(), &volumes).await;
    assert_eq!(load_streak(SYMBOLS[0]).await, Some(saved));

    // 隔天沒有成交量：外資比例條件不成立，只剩融資大增的 C；A 連買累加到 2 天。
    let (_, reports) =
        collect_reports(day(7), &symbols, &flows(), &margins(), &HashMap::new()).await;
    assert_eq!(reported(&reports), ["79976C"]);
    assert_eq!(load_streak(SYMBOLS[0]).await.map(|s| s.foreign), Some(2));

    clear_streaks().await;
}
