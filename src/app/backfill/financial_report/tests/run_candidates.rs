//! 單輪候選處理：各種結果的計數、單輪上限與季報期落後優先。

use super::*;

/// 假代號：F1 已有旗標、F2 Yahoo 成功、F3 Yahoo 404、F4 改用 BigGo、F5 兩邊都失敗、
/// F6 超過單輪上限不處理。
const SYMBOLS: [&str; 6] = [
    "79979F1", "79979F2", "79979F3", "79979F4", "79979F5", "79979F6",
];

async fn clear_flags() {
    for symbol in SYMBOLS {
        let _ = CLIENT.delete(&make_cache_key(symbol)).await;
    }
}

/// 旗標略過、Yahoo 成功、404、BigGo 備援、兩邊都失敗各自計數，實際嘗試數到上限就停。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn run_candidates_counts_each_outcome_and_stops_at_the_limit() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 run_candidates_counts_each_outcome_and_stops_at_the_limit：無 Redis 連線");
        return;
    }
    clear_flags().await;
    CLIENT
        .set(&make_cache_key("79979F1"), true, 60)
        .await
        .expect("預先寫入略過旗標");

    let candidates: Vec<String> = SYMBOLS.iter().map(|s| (*s).to_owned()).collect();
    let yahoo_calls = Mutex::new(Vec::new());
    let biggo_calls = Mutex::new(Vec::new());
    let repo = FakeRepository::default();
    let summary = run_candidates(
        &candidates,
        0,
        4,
        &repo,
        |symbol: &str| {
            yahoo_calls.lock().unwrap().push(symbol.to_owned());
            let result = match symbol {
                "79979F2" => Ok(reports(symbol)),
                "79979F3" => Err(anyhow::Error::new(YahooPageNotFoundError {
                    stock_symbol: symbol.to_owned(),
                    url: "https://example.invalid".to_owned(),
                })),
                _ => Err(anyhow!("Yahoo 測試失敗")),
            };
            async move { result }
        },
        |symbol: &str| {
            biggo_calls.lock().unwrap().push(symbol.to_owned());
            let result = match symbol {
                "79979F4" => Ok(reports(symbol)),
                _ => Ok(empty_reports()),
            };
            async move { result }
        },
        || async {},
    )
    .await
    .expect("run_candidates");

    assert_eq!(
        summary,
        RunSummary {
            candidates: 6,
            skipped: 1,
            prioritized: 0,
            succeeded: 1,
            fallback: 1,
            not_found: 1,
            failed: 1,
            rows: 2,
        }
    );
    assert_eq!(
        *yahoo_calls.lock().unwrap(),
        ["79979F2", "79979F3", "79979F4", "79979F5"]
    );
    assert_eq!(*biggo_calls.lock().unwrap(), ["79979F4", "79979F5"]);
    // 實際嘗試過的都寫了旗標；超過上限的 F6 沒有。
    for symbol in ["79979F2", "79979F3", "79979F4", "79979F5"] {
        assert!(
            CLIENT
                .get_bool(&make_cache_key(symbol))
                .await
                .unwrap_or(false),
            "{symbol}"
        );
    }
    assert!(
        !CLIENT
            .get_bool(&make_cache_key("79979F6"))
            .await
            .unwrap_or(false)
    );

    clear_flags().await;
}

/// 優先測試的假代號：P1 有每週旗標但仍要抓、P2 已有優先旗標、N1 有每週旗標、N2 一般輪替。
const PRIORITY_SYMBOLS: [&str; 4] = ["79979P1", "79979P2", "79979N1", "79979N2"];

async fn clear_priority_flags() {
    for symbol in PRIORITY_SYMBOLS {
        let _ = CLIENT.delete(&make_cache_key(symbol)).await;
        let _ = CLIENT.delete(&make_priority_cache_key(symbol)).await;
    }
}

/// 優先股票只看優先旗標、不看每週旗標，嘗試時兩個旗標都寫；其餘股票照每週旗標略過。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn run_candidates_prioritizes_lagging_stocks_past_the_weekly_flag() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!(
            "跳過 run_candidates_prioritizes_lagging_stocks_past_the_weekly_flag：無 Redis 連線"
        );
        return;
    }
    clear_priority_flags().await;
    for key in [
        make_cache_key("79979P1"),
        make_priority_cache_key("79979P2"),
        make_cache_key("79979N1"),
    ] {
        CLIENT.set(&key, true, 60).await.expect("預先寫入略過旗標");
    }

    let candidates: Vec<String> = PRIORITY_SYMBOLS.iter().map(|s| (*s).to_owned()).collect();
    let yahoo_calls = Mutex::new(Vec::new());
    let summary = run_candidates(
        &candidates,
        2,
        10,
        &FakeRepository::default(),
        |symbol: &str| {
            yahoo_calls.lock().unwrap().push(symbol.to_owned());
            let result = Ok(reports(symbol));
            async move { result }
        },
        |_symbol: &str| async { Ok(empty_reports()) },
        || async {},
    )
    .await
    .expect("run_candidates");

    assert_eq!(*yahoo_calls.lock().unwrap(), ["79979P1", "79979N2"]);
    assert_eq!(
        (summary.skipped, summary.prioritized, summary.succeeded),
        (2, 1, 2)
    );
    for key in [
        make_cache_key("79979P1"),
        make_priority_cache_key("79979P1"),
    ] {
        assert!(CLIENT.get_bool(&key).await.unwrap_or(false), "{key}");
    }
    assert!(
        !CLIENT
            .get_bool(&make_priority_cache_key("79979N2"))
            .await
            .unwrap_or(false),
        "一般輪替不寫優先旗標"
    );

    clear_priority_flags().await;
}

/// 只採集上市櫃普通股並依代號排序，落後的排最前面；查不到落後名單時照代號順序、沒有優先。
#[tokio::test]
async fn order_candidates_puts_lagging_stocks_first() {
    let stocks = [
        stock("2454", 2),
        stock("0050", 2),
        stock("1101", 2),
        stock("7708", 5),
        stock("8042", 4),
    ];
    let repo = FakeRepository {
        lagging: Some(vec!["8042".to_owned(), "7708".to_owned()]),
        ..Default::default()
    };
    assert_eq!(
        order_candidates(&repo, &stocks).await,
        (
            vec!["8042".to_owned(), "1101".to_owned(), "2454".to_owned()],
            1
        )
    );
    assert_eq!(
        order_candidates(&FakeRepository::default(), &stocks).await,
        (
            vec!["1101".to_owned(), "2454".to_owned(), "8042".to_owned()],
            0
        )
    );
}
