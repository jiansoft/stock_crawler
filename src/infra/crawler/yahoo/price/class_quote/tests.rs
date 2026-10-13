use rust_decimal::Decimal;

use super::*;

#[test]
fn test_build_class_quotes_api_url() {
    let category = YahooClassCategory::enabled(YahooClassExchange::Listed, 40, "半導體");

    assert_eq!(
        build_class_quotes_api_url(&category, 30),
        "https://tw.stock.yahoo.com/_td-stock/api/resource/StockServices.getClassQuotes;exchange=TAI;sectorId=40;offset=30"
    );
}

/// 驗證 `Referer` 指向該類股自己的頁面。
///
/// 這支 API 是類股頁的 XHR 端點；先前完全沒帶 `Referer` 送出，
/// 是被 Yahoo WAF 以 999 Request denied 擋下的可能原因之一。
#[test]
fn test_build_class_quotes_page_url() {
    let category = YahooClassCategory::enabled(YahooClassExchange::Listed, 21, "觀光餐旅");

    assert_eq!(
        build_class_quotes_page_url(&category),
        "https://tw.stock.yahoo.com/class-quote?sectorId=21&exchange=TAI"
    );
}

/// 驗證請求標頭帶齊 `Referer`、`Accept` 與 `Accept-Language`。
#[test]
fn test_build_class_quotes_headers() {
    let category = YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 40, "半導體");

    let headers = build_class_quotes_headers(&category).expect("標頭應可組出");

    assert_eq!(
        headers.get(header::REFERER).expect("必須帶 Referer"),
        "https://tw.stock.yahoo.com/class-quote?sectorId=40&exchange=TWO"
    );
    assert_eq!(
        headers.get(header::ACCEPT).expect("必須帶 Accept"),
        "application/json, text/plain, */*"
    );
    assert!(
        headers.contains_key(header::ACCEPT_LANGUAGE),
        "必須帶 Accept-Language"
    );
}

#[test]
fn category_key_uses_exchange_code_and_sector_id() {
    let category = YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 153, "半導體");

    assert_eq!(category_key(&category), "TWO:153");
}

/// 驗證盤中輪詢清單會排除認購 / 認售 / 指數類 / 公司債 / 牛熊證等不需採集的分類，
/// 只保留實際需要採集的分類。
#[test]
fn test_all_class_categories_excludes_disabled_categories() {
    let categories = all_class_categories();

    assert_eq!(categories.len(), 93);
    assert!(!categories.iter().any(|category| {
        matches!(
            (category.exchange, category.sector_id),
            (YahooClassExchange::Listed, 31..=33 | 49 | 51 | 52) // 排除指數類、回空資料的創新板(49)、牛熊證(51/52)
                | (YahooClassExchange::OverTheCounter, 33 | 163 | 165..=167) // 公司債(163)、認購/認售(165/166)、牛證(167)
        )
    }));
    assert!(categories.iter().any(|category| {
        category.exchange == YahooClassExchange::Listed && category.sector_id == 40
    }));
}

/// 專用 client 只建立一次，之後拿到的是同一個實例。
#[test]
fn class_quotes_client_is_built_once() {
    let first = get_yahoo_class_quotes_client().expect("可建立 client");
    let second = get_yahoo_class_quotes_client().expect("可重用 client");
    assert!(std::ptr::eq(first, second));
}

fn semiconductor() -> YahooClassCategory {
    YahooClassCategory::enabled(YahooClassExchange::Listed, 40, "半導體")
}

/// 組出一頁類股 API 回應；`next` 為下一頁 offset（字串，與 Yahoo 相同）。
fn page_body(symbols: &[&str], total: usize, next: Option<&str>) -> Vec<u8> {
    let list: Vec<serde_json::Value> = symbols
        .iter()
        .map(|symbol| serde_json::json!({"systexId": symbol, "price": {"raw": "100"}}))
        .collect();
    serde_json::to_vec(&serde_json::json!({
        "list": list,
        "pagination": {"resultsTotal": total, "nextOffset": next}
    }))
    .expect("序列化")
}

fn page(symbols: &[&str], total: usize, next: Option<usize>) -> ClassQuotesPageResult {
    parse_class_quotes_page(
        &semiconductor(),
        0,
        "test-url",
        StatusCode::OK,
        &page_body(
            symbols,
            total,
            next.map(|offset| offset.to_string()).as_deref(),
        ),
    )
    .expect("可解析")
}

/// 單頁解析：快照、原始筆數、總筆數與下一頁 offset。
#[test]
fn parse_page_reads_items_and_pagination() {
    let result = page(&["2330", "2303"], 45, Some(30));
    assert_eq!(result.snapshots.len(), 2);
    assert_eq!(result.snapshots[0].0, "2330");
    assert_eq!(
        (
            result.raw_item_count,
            result.results_total,
            result.next_offset
        ),
        (2, 45, Some(30))
    );
}

/// HTTP 失敗、JSON 壞掉、nextOffset 不是數字、非首頁空列表卻有總筆數都回錯誤；
/// 首頁空列表不在這裡報錯（由分頁迴圈判斷）。
#[test]
fn parse_page_rejects_bad_responses() {
    let category = semiconductor();
    let body = page_body(&["2330"], 1, None);
    let parse = |offset, status, body: &[u8]| {
        parse_class_quotes_page(&category, offset, "test-url", status, body)
    };
    let error = parse(0, StatusCode::FORBIDDEN, b"denied")
        .err()
        .expect("403 應失敗");
    assert!(error.to_string().contains("403"), "{error}");
    assert!(parse(0, StatusCode::OK, b"<html>").is_err());
    assert!(parse(0, StatusCode::OK, &page_body(&["2330"], 1, Some("abc"))).is_err());
    assert!(parse(30, StatusCode::OK, &page_body(&[], 45, None)).is_err());
    let first_empty =
        parse(0, StatusCode::OK, &page_body(&[], 0, None)).expect("首頁空不在這裡報錯");
    assert_eq!(first_empty.raw_item_count, 0);
    assert!(parse(0, StatusCode::OK, &body).is_ok());
}

/// 分頁迴圈：依 nextOffset 往下抓、頁與頁之間停頓、重複代號以後者為準、診斷數字正確。
#[tokio::test]
async fn collect_pages_follows_next_offset() {
    let pauses = std::sync::Mutex::new(0);
    let requested = std::sync::Mutex::new(Vec::new());
    let result = collect_category_pages(
        &semiconductor(),
        |offset| {
            requested.lock().unwrap().push(offset);
            async move {
                Ok(match offset {
                    0 => page(&["2330", "2303"], 3, Some(2)),
                    _ => page(&["2303", "2454"], 3, None),
                })
            }
        },
        || {
            *pauses.lock().unwrap() += 1;
            async {}
        },
    )
    .await
    .expect("可抓完");
    assert_eq!(*requested.lock().unwrap(), vec![0, 2]);
    assert_eq!(*pauses.lock().unwrap(), 1);
    assert_eq!(
        result.diagnostics,
        CategoryFetchDiagnostics {
            page_count: 2,
            raw_item_count: 4,
            snapshot_count: 3
        }
    );
    assert!(result.snapshots.contains_key("2454"));
}

/// nextOffset 沒有往前推進時停止，避免無窮迴圈。
#[tokio::test]
async fn collect_pages_stops_when_offset_does_not_advance() {
    let result = collect_category_pages(
        &semiconductor(),
        |_| async { Ok(page(&["2330"], 1, Some(0))) },
        || async {},
    )
    .await
    .expect("可抓完");
    assert_eq!(result.diagnostics.page_count, 1);
}

/// 首頁就空、或整個類股解析不出任何股票，都要回錯誤。
#[tokio::test]
async fn collect_pages_rejects_empty_category() {
    let empty_first = collect_category_pages(
        &semiconductor(),
        |_| async { Ok(page(&[], 0, None)) },
        || async {},
    )
    .await;
    assert!(
        empty_first
            .err()
            .is_some_and(|e| e.to_string().contains("首頁為空"))
    );

    let unparsable = collect_category_pages(
        &semiconductor(),
        |_| async {
            Ok(ClassQuotesPageResult {
                snapshots: Vec::new(),
                raw_item_count: 2,
                results_total: 2,
                next_offset: None,
            })
        },
        || async {},
    )
    .await;
    assert!(
        unparsable
            .err()
            .is_some_and(|e| e.to_string().contains("未解析出任何股票"))
    );
}

/// Live 測試：驗證單一類股能抓到完整分頁資料，而不只首頁前 30 筆。
#[tokio::test]
#[ignore]
async fn test_fetch_category_snapshots_integration() {
    let category = YahooClassCategory::enabled(YahooClassExchange::Listed, 40, "半導體");

    let snapshots = fetch_category_snapshots(&category).await.unwrap().snapshots;

    dbg!(&snapshots);

    assert!(
        snapshots.len() > 30,
        "expected more than first page of Yahoo class quotes"
    );

    let snapshot = snapshots.get("2330").expect("expected 2330 in TAI 40");
    assert_eq!(snapshot.symbol, "2330");
    assert_eq!(snapshot.name, "台積電");
    assert!(snapshot.price > Decimal::ZERO);
}

/// Live 測試：驗證上櫃半導體類股能抓到完整分頁資料。
#[tokio::test]
#[ignore]
async fn test_fetch_otc_category_snapshots_integration() {
    let category = YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 153, "半導體");

    let snapshots = fetch_category_snapshots(&category).await.unwrap().snapshots;

    assert!(
        !snapshots.is_empty(),
        "expected some snapshots for Yahoo OTC class quotes"
    );
}

/// Live 測試：列出上市、上櫃、興櫃三個半導體類股下的所有股票資訊。
#[tokio::test]
#[ignore]
async fn test_fetch_all_semiconductor_categories_integration() {
    let categories = [
        YahooClassCategory::enabled(YahooClassExchange::Listed, 40, "半導體"),
        YahooClassCategory::enabled(YahooClassExchange::OverTheCounter, 153, "半導體"),
        YahooClassCategory::enabled(YahooClassExchange::Emerging, 99311, "半導體"),
    ];

    for category in categories {
        let snapshots = fetch_category_snapshots(&category).await.unwrap().snapshots;
        let mut rows: Vec<_> = snapshots.into_iter().collect();
        rows.sort_by(|left, right| left.0.cmp(&right.0));

        println!(
            "\n=== {} 半導體 ({}) 共 {} 檔 ===",
            category.exchange.label(),
            category.sector_id,
            rows.len()
        );

        for (symbol, snapshot) in rows {
            println!(
                "{}\t{}\tprice={}\tchange={}\trange={}%\topen={}\thigh={}\tlow={}\tlast_close={}\tvolume_k={}",
                symbol,
                snapshot.name,
                snapshot.price,
                snapshot.change,
                snapshot.change_range,
                snapshot.open,
                snapshot.high,
                snapshot.low,
                snapshot.last_close,
                snapshot.volume
            );
        }
    }
}
