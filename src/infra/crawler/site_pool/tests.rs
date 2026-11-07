//! 報價站點池的測試：輪詢游標、站點略過與備援順序、各取價入口。

use super::*;

/// 驗證全域輪詢游標在不同站點池大小間切換時不會產生越界索引。
#[test]
fn test_get_and_increment_index_supports_different_pool_sizes() {
    INDEX.store(0, Ordering::SeqCst);

    assert_eq!(get_and_increment_index(9), 0);
    assert_eq!(get_and_increment_index(8), 1);

    INDEX.store(8, Ordering::SeqCst);
    assert_eq!(get_and_increment_index(8), 0);
    assert_eq!(get_and_increment_index(9), 0);
}

fn ok_price<'a>(_: &'a str) -> registry::SiteFuture<'a, Decimal> {
    Box::pin(async { Ok(Decimal::new(12340, 2)) })
}

fn fail_price<'a>(stock_symbol: &'a str) -> registry::SiteFuture<'a, Decimal> {
    Box::pin(async move { Err(anyhow!("{stock_symbol} 查無報價")) })
}

fn skip_price<'a>(_: &'a str) -> registry::SiteFuture<'a, Decimal> {
    Box::pin(async { Err(SiteSkipped::new("local rate limit".to_owned()).into()) })
}

fn ok_quotes<'a>(stock_symbol: &'a str) -> registry::SiteFuture<'a, declare::StockQuotes> {
    Box::pin(async move {
        Ok(declare::StockQuotes {
            stock_symbol: stock_symbol.to_owned(),
            price: 101.5,
            change: 1.5,
            change_range: 1.5,
        })
    })
}

fn fail_quotes<'a>(_: &'a str) -> registry::SiteFuture<'a, declare::StockQuotes> {
    Box::pin(async { Err(anyhow!("欄位格式不符")) })
}

/// 失敗的站點換下一站；回傳命中的站點並把價格標準化。
///
/// 起始站點由全域遊標決定，不論從哪一站開始，結果都要是成功的那一站。
/// 站點名稱各測試獨立，避免健康紀錄（熔斷）互相影響。
#[tokio::test]
async fn price_pool_falls_back_to_next_site_and_normalizes() {
    let sites = [
        PriceSite {
            name: "TestPriceDown",
            fetch: fail_price,
        },
        PriceSite {
            name: "TestPriceUp",
            fetch: ok_price,
        },
    ];
    for _ in 0..sites.len() {
        let fetched = fetch_stock_price_from_site_pool("2330", &sites, "test pool")
            .await
            .expect("有一站成功就要回傳");
        assert_eq!(fetched.site_name, "TestPriceUp");
        assert_eq!(fetched.price.to_string(), "123.4");
    }
}

/// 站點主動略過（本地限流）不算失敗：連續略過遠超過熔斷門檻也不會被熔斷，
/// 對照組連續失敗同樣次數就會熔斷、不再出現在嘗試順序裡。
#[tokio::test]
async fn skipped_site_never_trips_while_failing_site_does() {
    let skipping = [
        PriceSite {
            name: "TestPriceSkip",
            fetch: skip_price,
        },
        PriceSite {
            name: "TestPriceSkipFallback",
            fetch: ok_price,
        },
    ];
    let failing = [
        PriceSite {
            name: "TestPriceSkipControl",
            fetch: fail_price,
        },
        PriceSite {
            name: "TestPriceSkipControlFallback",
            fetch: ok_price,
        },
    ];
    // 每次都從第 0 站開始，確保每輪都會先試到略過／失敗的那一站。
    for _ in 0..12 {
        INDEX.store(0, Ordering::SeqCst);
        let fetched = fetch_stock_price_from_site_pool("2330", &skipping, "test pool")
            .await
            .expect("略過後要換下一站");
        assert_eq!(fetched.site_name, "TestPriceSkipFallback");
        INDEX.store(0, Ordering::SeqCst);
        fetch_stock_price_from_site_pool("2330", &failing, "test pool")
            .await
            .expect("失敗後要換下一站");
    }

    let skip_names = skipping.map(|site| site.name);
    assert!(
        plan_attempt_order(&skip_names, 0).contains(&0),
        "略過不累積連續失敗，不能被熔斷"
    );
    let fail_names = failing.map(|site| site.name);
    assert!(
        !plan_attempt_order(&fail_names, 0).contains(&0),
        "對照組連續失敗 12 次必須熔斷"
    );
}

/// 全部站點都略過或失敗時，錯誤訊息照樣列出略過的原因。
#[tokio::test]
async fn price_pool_reports_skip_reason_when_nothing_succeeds() {
    let sites = [PriceSite {
        name: "TestPriceSkipOnly",
        fetch: skip_price,
    }];
    let message = fetch_stock_price_from_site_pool("2330", &sites, "test pool")
        .await
        .expect_err("沒有站點成功要回錯誤")
        .to_string();
    assert_eq!(
        message,
        "Failed to fetch stock price(2330) from test pool: TestPriceSkipOnly: local rate limit"
    );
    assert!(is_skipped(&SiteSkipped::new("x".to_owned()).into()));
    assert!(!is_skipped(&anyhow!("x")));
}

/// 對外四個入口各自走正式站點清單：所有站點都被導到沒有設定任何回應的 mock（一律 404）時，
/// 錯誤訊息要列出該清單的每一站，證明每站都試過一次、沒有連到外網。
#[tokio::test]
async fn public_entries_try_every_registered_site() {
    let server = wiremock::MockServer::start().await;
    let base = server.uri();
    let origins = [
        "https://tw.stock.yahoo.com",
        "https://api.fugle.tw",
        "https://www.nstock.tw",
        "https://ws.api.cnyes.com",
        "https://api.biggo.com",
    ]
    .map(|origin| (origin, base.as_str()));

    let (all, backup, backup_with_source, quotes) =
        http::mock_origin::with_origins(&origins, async {
            (
                fetch_stock_price_from_remote_site("79981").await,
                fetch_stock_price_from_backup_sites("79981").await,
                fetch_stock_price_from_backup_sites_with_source("79981").await,
                fetch_stock_quotes_from_remote_site("79981").await,
            )
        })
        .await;

    let expect_every_site = |message: String, scope: &str, names: Vec<&str>| {
        assert!(message.contains(scope), "{message}");
        for name in names {
            assert!(
                message.contains(&format!("{name}: ")),
                "缺 {name}：{message}"
            );
        }
    };
    let all_names = ALL_PRICE_SITES.iter().map(|site| site.name).collect();
    let backup_names = || BACKUP_PRICE_SITES.iter().map(|site| site.name).collect();
    let quote_names = ALL_QUOTE_SITES.iter().map(|site| site.name).collect();
    expect_every_site(
        all.expect_err("全部 404 要回錯誤").to_string(),
        "stock price(79981) from all sites",
        all_names,
    );
    expect_every_site(
        backup.expect_err("全部 404 要回錯誤").to_string(),
        "stock price(79981) from backup sites",
        backup_names(),
    );
    expect_every_site(
        backup_with_source
            .expect_err("全部 404 要回錯誤")
            .to_string(),
        "stock price(79981) from backup sites",
        backup_names(),
    );
    expect_every_site(
        quotes.expect_err("全部 404 要回錯誤").to_string(),
        "stock quotes(79981) from",
        quote_names,
    );
}

/// 全部站點失敗時，錯誤訊息列出每一站的原因。
#[tokio::test]
async fn price_pool_reports_every_site_when_all_fail() {
    let sites = [
        PriceSite {
            name: "TestPriceDownA",
            fetch: fail_price,
        },
        PriceSite {
            name: "TestPriceDownB",
            fetch: fail_price,
        },
    ];
    let message = fetch_stock_price_from_site_pool("2330", &sites, "test pool")
        .await
        .expect_err("全部失敗要回錯誤")
        .to_string();
    assert!(
        message.starts_with("Failed to fetch stock price(2330) from test pool: "),
        "{message}"
    );
    assert!(
        message.contains("TestPriceDownA: 2330 查無報價"),
        "{message}"
    );
    assert!(
        message.contains("TestPriceDownB: 2330 查無報價"),
        "{message}"
    );
}

/// 完整報價走同一套輪詢；全部失敗時錯誤訊息標明是 stock quotes。
#[tokio::test]
async fn quote_pool_falls_back_and_reports_failures() {
    let sites = [
        QuoteSite {
            name: "TestQuoteDown",
            fetch: fail_quotes,
        },
        QuoteSite {
            name: "TestQuoteUp",
            fetch: ok_quotes,
        },
    ];
    let quotes = fetch_stock_quotes_from_site_pool("1101", &sites, "test pool")
        .await
        .expect("有一站成功就要回傳");
    assert_eq!(quotes.stock_symbol, "1101");
    assert_eq!(quotes.price, 101.5);

    let failing = [QuoteSite {
        name: "TestQuoteDownOnly",
        fetch: fail_quotes,
    }];
    let message = fetch_stock_quotes_from_site_pool("1101", &failing, "test pool")
        .await
        .expect_err("全部失敗要回錯誤")
        .to_string();
    assert_eq!(
        message,
        "Failed to fetch stock quotes(1101) from test pool: TestQuoteDownOnly: 欄位格式不符"
    );
}

/// 驗證完整站點池可以成功抓取多檔股票的最新成交價。
///
/// 此測試會實際連線外部站點，主要用於手動驗證輪詢與備援流程。
#[tokio::test]
#[ignore]
async fn test_fetch_stock_price_from_remote_site() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_price");

    let sites = [
        "2330", "1101", "1232", "1303", "1326", "3008", "9941", "2912",
    ];

    for site in sites {
        match fetch_stock_price_from_remote_site(site).await {
            Ok(e) => {
                //dbg!(e);
                println!("{}:{}", site, e);
            }
            Err(why) => {
                tracing::debug!("Failed to fetch_price because {:?}", why);
            }
        }
    }
    flush_site_latency_stats();
    tracing::debug!("結束 fetch_price");
}

/// 驗證完整站點池可以成功抓取多檔股票的完整報價資訊。
///
/// 此測試會實際連線外部站點，主要用於手動驗證完整報價輪詢流程。
#[tokio::test]
#[ignore]
async fn test_fetch_stock_quotes_from_remote_site() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 fetch_stock_quotes_from_remote_site");

    let sites = [
        "2330", "1101", "1232", "1303", "1326", "3008", "9941", "2912",
    ];

    for site in sites {
        match fetch_stock_quotes_from_remote_site(site).await {
            Ok(e) => {
                //dbg!(e);
                println!("{}:{:?}", site, e);
            }
            Err(why) => {
                tracing::debug!("Failed to fetch_price because {:?}", why);
            }
        }
    }

    tracing::debug!("結束 fetch_stock_quotes_from_remote_site");
}
