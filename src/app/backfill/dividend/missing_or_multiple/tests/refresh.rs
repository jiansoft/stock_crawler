//! 逐檔補抓迴圈（`refresh_stocks`／`refresh_year`）：跳過旗標、額度用完延後、關機提早結束。

use super::*;

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

/// Yahoo 404 第一次沿用一般略過時間（可能是暫時性，H37），60 天內再次 404 才拉長到 30 天；
/// 補抓成功會清掉「曾經 404」標記。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（Redis），請加 --features integration-tests 執行"
)]
async fn refresh_stocks_extends_skip_only_on_repeated_not_found() {
    dotenvy::dotenv().ok();
    if CLIENT.ping().await.is_err() {
        println!("跳過 refresh_stocks_extends_skip_only_on_repeated_not_found：無 Redis 連線");
        return;
    }
    let not_found = "79979DV3".to_owned();
    let ok = "79979DV2".to_owned();
    let marker = |symbol: &str| format!("{}:not_found", make_cache_key(symbol));
    let clear = || async {
        for symbol in [&not_found, &ok] {
            let _ = CLIENT.delete(&make_cache_key(symbol)).await;
            let _ = CLIENT.delete(&marker(symbol)).await;
        }
    };
    clear().await;
    CLIENT
        .set(marker(&ok), true, 60)
        .await
        .expect("預先寫入曾經 404 的標記");

    let symbols = vec![ok.clone(), not_found.clone()];
    let backfill = |symbol: &str| {
        let result = if symbol == "79979DV3" {
            Err(anyhow::Error::new(
                yahoo::dividend::YahooPageNotFoundError {
                    stock_symbol: symbol.to_owned(),
                    url: "https://example.invalid".to_owned(),
                },
            ))
        } else {
            Ok(())
        };
        async move { result }
    };
    refresh_stocks(&symbols, 2, backfill, || async {}, || false)
        .await
        .expect("第一輪");
    let first_ttl = CLIENT.ttl(&make_cache_key(&not_found)).await.expect("ttl");
    let ok_marker_after_success = CLIENT.ttl(&marker(&ok)).await.expect("ttl");
    // 模擬一般略過時間到期，下一輪又排到這檔。
    let _ = CLIENT.delete(&make_cache_key(&not_found)).await;
    refresh_stocks(&symbols[1..], 1, backfill, || async {}, || false)
        .await
        .expect("第二輪");
    let second_ttl = CLIENT.ttl(&make_cache_key(&not_found)).await.expect("ttl");
    clear().await;

    let normal_max = REFRESH_SKIP_TTL_MAX_SECONDS as i64;
    assert!(
        (1..=normal_max).contains(&first_ttl),
        "第一次 404 只用一般略過時間：{first_ttl}"
    );
    assert!(
        second_ttl > normal_max,
        "再次 404 拉長到 30 天：{second_ttl}"
    );
    assert_eq!(ok_marker_after_success, -2, "補抓成功要清掉標記");
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
