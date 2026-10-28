//! 年化報酬倉儲的排行榜、覆蓋率與補算日期測試。

use super::*;

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_ranking_and_coverage() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_fetch_ranking_and_coverage：無資料庫連接");
        return;
    }

    let repo = PgCagrRepository::new();
    cleanup().await;

    // 三筆可算（年化 5 / 15 / -3）＋ 一筆資料不足（同時標記異常）。
    let records = vec![
        complete_record(FAKE_SYMBOL, CagrPeriod::Y1, dec!(5)),
        complete_record(FAKE_SYMBOL_B, CagrPeriod::Y1, dec!(15)),
        complete_record(FAKE_SYMBOL_C, CagrPeriod::Y1, dec!(-3)),
    ];
    repo.save_batch(&records).await.expect("save_batch");
    // 再寫入資料不足的列：Y1 一筆（驗證排行榜排除它）、Y2 兩筆（驗證涵蓋率計數）。
    let incompletes = vec![
        incomplete_record(FAKE_SYMBOL_D, CagrPeriod::Y1),
        incomplete_record(FAKE_SYMBOL, CagrPeriod::Y2),
        incomplete_record(FAKE_SYMBOL_C, CagrPeriod::Y2),
    ];
    repo.save_batch(&incompletes).await.expect("save_batch");

    // 排行榜：依 total 口徑由高至低，且不含資料不足者。
    let ranking = repo
        .fetch_ranking(test_date(), CagrPeriod::Y1, CagrMetric::Total, 10, 0)
        .await
        .expect("fetch_ranking");
    let symbols: Vec<&str> = ranking
        .iter()
        .map(|item| item.stock_symbol.as_str())
        .collect();
    assert_eq!(
        symbols,
        vec![FAKE_SYMBOL_B, FAKE_SYMBOL, FAKE_SYMBOL_C],
        "應依年化報酬率由高至低，且不含 data_complete = false 的 {FAKE_SYMBOL_D}"
    );
    assert!(ranking.iter().all(|item| item.data_complete));

    // offset/limit 亦應生效。
    let paged = repo
        .fetch_ranking(test_date(), CagrPeriod::Y1, CagrMetric::Total, 1, 1)
        .await
        .expect("fetch_ranking paged");
    assert_eq!(paged.len(), 1);
    assert_eq!(paged[0].stock_symbol, FAKE_SYMBOL);

    // 涵蓋率統計：Y1 期間共 4 列，3 列可算（正報酬 2 檔），1 列資料不足且標記異常。
    let coverage = repo
        .fetch_coverage(test_date(), CagrPeriod::Y1, CagrMetric::Total)
        .await
        .expect("fetch_coverage");
    assert_eq!(coverage.universe, 4);
    assert_eq!(coverage.counted, 3);
    assert_eq!(coverage.incomplete, 1);
    assert_eq!(coverage.anomaly_flagged, 1);
    assert_eq!(coverage.positive, 2);

    // Y2 期間共 2 列，皆為資料不足且標記異常。
    let coverage = repo
        .fetch_coverage(test_date(), CagrPeriod::Y2, CagrMetric::Total)
        .await
        .expect("fetch_coverage");
    assert_eq!(coverage.universe, 2);
    assert_eq!(coverage.counted, 0);
    assert_eq!(coverage.incomplete, 2);
    assert_eq!(coverage.anomaly_flagged, 2);
    assert_eq!(coverage.positive, 0);

    // 最新基準日必定不早於測試用的 1990-01-02。
    let latest = repo.fetch_latest_date().await.expect("fetch_latest_date");
    assert!(latest.is_some_and(|d| d >= test_date()));

    cleanup().await;
}

/// 新增期間後的回填依據：找出「已有結果、但缺少該期間」的基準日。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_dates_missing_period() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_fetch_dates_missing_period：無資料庫連接");
        return;
    }

    let repo = PgCagrRepository::new();
    cleanup().await;

    // 只有 Y1 的基準日 —— 缺 Y7，必須被列為待回填。
    repo.save_batch(&[complete_record(FAKE_SYMBOL, CagrPeriod::Y1, dec!(5))])
        .await
        .expect("save_batch");
    let pending = repo
        .fetch_dates_missing_period(CagrPeriod::Y7)
        .await
        .expect("fetch_dates_missing_period");
    assert!(pending.contains(&test_date()));
    assert!(
        pending.windows(2).all(|pair| pair[0] < pair[1]),
        "回傳的基準日必須由早至晚且不重複"
    );

    // 補上 Y7 之後該日期就不該再出現。
    repo.save_batch(&[complete_record(FAKE_SYMBOL, CagrPeriod::Y7, dec!(5))])
        .await
        .expect("save_batch Y7");
    let pending = repo
        .fetch_dates_missing_period(CagrPeriod::Y7)
        .await
        .expect("fetch_dates_missing_period again");
    assert!(!pending.contains(&test_date()));

    cleanup().await;
}

/// 排行榜分頁查詢：全市場名次、資料不足殿後、篩選與分頁語意。
///
/// 名次必須是「未套用篩選的全市場名次」，因此以產業／關鍵字篩選後，
/// 同一檔股票的 `rank` 不可改變——這是前端把 rank 當成股票穩定屬性的
/// 前提。測試自行寫入四檔假股票以滿足 JOIN，只在遠早於本功能上線的
/// 1990-01-02 寫入計算結果，結束後母檔與結果一律清除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_ranking_page_semantics() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_fetch_ranking_page_semantics：無資料庫連接");
        return;
    }

    let symbols = fake_symbols();
    let industry = TEST_INDUSTRY;
    let other_industry = Some(OTHER_INDUSTRY);

    let cleanup_borrowed = || async {
        cleanup().await;
        cleanup_stocks().await;
    };
    cleanup_borrowed().await;
    seed_stocks().await;

    let repo = PgCagrRepository::new();
    // 年化 30 / 20 / 10 三檔可算，第四檔資料不足。
    let records = vec![
        complete_record(&symbols[0], CagrPeriod::Y1, dec!(30)),
        complete_record(&symbols[1], CagrPeriod::Y1, dec!(20)),
        complete_record(&symbols[2], CagrPeriod::Y1, dec!(10)),
        incomplete_record(&symbols[3], CagrPeriod::Y1),
    ];
    repo.save_batch(&records).await.expect("save_batch");

    let base = CagrRankingQuery::new(test_date(), CagrPeriod::Y1);
    let page = repo
        .fetch_ranking_page(&base)
        .await
        .expect("fetch_ranking_page");
    let listed: Vec<(&str, Option<i64>)> = page
        .items
        .iter()
        .filter(|item| symbols.contains(&item.cagr.stock_symbol))
        .map(|item| (item.cagr.stock_symbol.as_str(), item.rank))
        .collect();
    assert_eq!(listed.len(), 4, "資料不足者也必須出現在清單中");
    // 可算三檔依年化由高至低，且名次遞增；資料不足者 rank 為 None。
    assert_eq!(listed[0].0, symbols[0]);
    assert_eq!(listed[1].0, symbols[1]);
    assert_eq!(listed[2].0, symbols[2]);
    assert_eq!(listed[3].0, symbols[3]);
    assert!(listed[3].1.is_none(), "資料不足不得佔名次");
    let ranks: Vec<i64> = listed[..3].iter().filter_map(|(_, rank)| *rank).collect();
    assert_eq!(ranks.len(), 3);
    assert!(ranks[0] < ranks[1] && ranks[1] < ranks[2]);
    assert_eq!(page.items.len() as i64, page.total.min(base.limit));
    assert!(page.items.iter().all(|item| !item.name.is_empty()));

    // 產業篩選：名次不得重編，仍是全市場名次。
    let filtered = repo
        .fetch_ranking_page(&CagrRankingQuery {
            industry_id: Some(industry),
            ..base.clone()
        })
        .await
        .expect("fetch_ranking_page industry");
    let filtered_ranks: Vec<Option<i64>> = filtered
        .items
        .iter()
        .filter(|item| symbols.contains(&item.cagr.stock_symbol))
        .map(|item| item.rank)
        .collect();
    assert_eq!(
        filtered_ranks,
        listed.iter().map(|(_, rank)| *rank).collect::<Vec<_>>(),
        "套用篩選後名次必須維持全市場名次"
    );
    assert!(filtered.total <= page.total);
    assert_eq!(filtered.coverage, page.coverage, "涵蓋統計不隨畫面篩選變動");

    // 其他產業必然不含這四檔。
    if let Some(other) = other_industry {
        let elsewhere = repo
            .fetch_ranking_page(&CagrRankingQuery {
                industry_id: Some(other),
                ..base.clone()
            })
            .await
            .expect("fetch_ranking_page other industry");
        assert!(
            elsewhere
                .items
                .iter()
                .all(|item| !symbols.contains(&item.cagr.stock_symbol))
        );
    }

    // include_incomplete = false 時資料不足者整列消失，total 同步變小。
    let complete_only = repo
        .fetch_ranking_page(&CagrRankingQuery {
            include_incomplete: false,
            ..base.clone()
        })
        .await
        .expect("fetch_ranking_page complete only");
    assert!(
        complete_only
            .items
            .iter()
            .all(|item| item.rank.is_some() && item.cagr.data_complete)
    );
    assert!(complete_only.total < page.total);

    // 關鍵字比對代號；分頁位移超出範圍時 total 仍須正確。
    let keyword = repo
        .fetch_ranking_page(&CagrRankingQuery {
            keyword: Some(symbols[0].clone()),
            ..base.clone()
        })
        .await
        .expect("fetch_ranking_page keyword");
    assert!(
        keyword
            .items
            .iter()
            .any(|item| item.cagr.stock_symbol == symbols[0])
    );
    let beyond = repo
        .fetch_ranking_page(&CagrRankingQuery {
            offset: page.total + 100,
            ..base.clone()
        })
        .await
        .expect("fetch_ranking_page beyond");
    assert!(beyond.items.is_empty());
    assert_eq!(beyond.total, page.total, "位移超界不可讓 total 退化成 0");

    cleanup_borrowed().await;
}
