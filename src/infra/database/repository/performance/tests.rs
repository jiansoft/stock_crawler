use super::*;
use crate::domain::performance::entity::SimulationOutcome;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// 測試專用的假代號前綴；真實市場不存在此代號。
const FAKE_SYMBOL: &str = "79979";
/// 排行榜測試用的另外兩個假代號。
const FAKE_SYMBOL_B: &str = "79979B";
const FAKE_SYMBOL_C: &str = "79979C";
const FAKE_SYMBOL_D: &str = "79979D";

/// 測試基準日。
///
/// 刻意選 1990-01-02 這種遠早於本功能上線的日期：`fetch_coverage`／
/// `fetch_ranking` 是以 (date, period) 為範圍的整體統計，若沿用近期日期，
/// 正式資料會混進計數而讓斷言時好時壞。
fn test_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(1990, 1, 2).unwrap()
}

/// 建立一筆資料齊全的測試資料。
fn complete_record(symbol: &str, period: CagrPeriod, cagr: Decimal) -> DomainStockCagr {
    let outcome = SimulationOutcome {
        end_shares: dec!(100.5),
        cash_received: dec!(250.0),
        end_value: dec!(12000.0),
        total_return_pct: dec!(20.0),
        cagr_pct: cagr,
    };

    DomainStockCagr {
        date: test_date(),
        stock_symbol: symbol.to_string(),
        period,
        base_date: NaiveDate::from_ymd_opt(1989, 1, 3),
        base_price: Some(dec!(100.0)),
        end_price: Some(dec!(119.4)),
        years: Some(dec!(1.0)),
        price: Some(SimulationOutcome {
            cash_received: Decimal::ZERO,
            cagr_pct: cagr - dec!(1),
            ..outcome
        }),
        total: Some(outcome),
        reinvested: Some(SimulationOutcome {
            cash_received: Decimal::ZERO,
            cagr_pct: cagr + dec!(1),
            ..outcome
        }),
        first_quote_date: NaiveDate::from_ymd_opt(1988, 1, 4),
        shortfall_days: Some(0),
        data_complete: true,
        has_anomaly: false,
        dividend_events: 2,
    }
}

/// 建立一筆資料不足的測試資料（所有數值欄位皆為 None）。
fn incomplete_record(symbol: &str, period: CagrPeriod) -> DomainStockCagr {
    DomainStockCagr {
        date: test_date(),
        stock_symbol: symbol.to_string(),
        period,
        base_date: None,
        base_price: None,
        end_price: None,
        years: None,
        price: None,
        total: None,
        reinvested: None,
        first_quote_date: NaiveDate::from_ymd_opt(1989, 6, 1),
        shortfall_days: None,
        data_complete: false,
        has_anomaly: true,
        dividend_events: 0,
    }
}

/// 四個假代號，供排行榜測試建立完整母體。
fn fake_symbols() -> Vec<String> {
    vec![
        FAKE_SYMBOL.to_string(),
        FAKE_SYMBOL_B.to_string(),
        FAKE_SYMBOL_C.to_string(),
        FAKE_SYMBOL_D.to_string(),
    ]
}

/// 排行榜測試用的產業編號（`stock_industry.sql` 已預先建立 1 與 2）。
const TEST_INDUSTRY: i32 = 1;
const OTHER_INDUSTRY: i32 = 2;

/// 清理測試寫入的資料列；測試結束務必呼叫，避免污染資料庫。
async fn cleanup() {
    let _ = sqlx::query("DELETE FROM stock_cagr WHERE stock_symbol = ANY($1)")
        .bind(fake_symbols())
        .execute(database::get_connection())
        .await;
}

/// 建立排行榜測試所需的假股票母檔。
///
/// 排行榜查詢要 JOIN `stocks` 取名稱與產業，早期版本改為借用資料庫既有的
/// 真實代號，但 CI 的測試資料庫只有結構沒有資料，測試因此永遠跳過 ——
/// 那些篩選、分頁與名次語意等於從未被驗證。改為自行寫入假代號後，
/// 空資料庫也能完整執行，且不會在正式資料的代號上留下痕跡。
async fn seed_stocks() {
    for (index, symbol) in fake_symbols().iter().enumerate() {
        let _ = sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_industry_id, "SuspendListing")
               VALUES ($1, $2, $1, $3, false)
               ON CONFLICT (stock_symbol) DO UPDATE
                   SET "Name" = excluded."Name", stock_industry_id = excluded.stock_industry_id"#,
        )
        .bind(symbol)
        .bind(format!("測試股{}", index + 1))
        .bind(TEST_INDUSTRY)
        .execute(database::get_connection())
        .await;
    }
}

/// 移除假股票母檔。
async fn cleanup_stocks() {
    let _ = sqlx::query("DELETE FROM stocks WHERE stock_symbol = ANY($1)")
        .bind(fake_symbols())
        .execute(database::get_connection())
        .await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_save_batch_round_trip_and_idempotent() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_save_batch_round_trip_and_idempotent：無資料庫連接");
        return;
    }

    let repo = PgCagrRepository::new();
    cleanup().await;

    // 1. 寫入後讀回應一致
    let record = complete_record(FAKE_SYMBOL, CagrPeriod::Y1, dec!(12.3456));
    let affected = repo
        .save_batch(std::slice::from_ref(&record))
        .await
        .expect("save_batch");
    assert_eq!(affected, 1);

    let fetched = repo
        .fetch_by_symbol(test_date(), FAKE_SYMBOL)
        .await
        .expect("fetch_by_symbol");
    assert_eq!(fetched.len(), 1);
    let first = &fetched[0];
    assert_eq!(first.stock_symbol, record.stock_symbol);
    assert_eq!(first.period, CagrPeriod::Y1);
    assert_eq!(first.base_date, record.base_date);
    assert_eq!(first.total, record.total);
    assert_eq!(first.reinvested, record.reinvested);
    assert!(first.data_complete);
    assert_eq!(first.dividend_events, 2);

    // 2. 同一主鍵重複寫入是覆蓋而非新增
    let mut updated = complete_record(FAKE_SYMBOL, CagrPeriod::Y1, dec!(99.9999));
    updated.dividend_events = 7;
    repo.save_batch(&[updated]).await.expect("save_batch again");

    let fetched = repo
        .fetch_by_symbol(test_date(), FAKE_SYMBOL)
        .await
        .expect("fetch_by_symbol");
    assert_eq!(fetched.len(), 1, "重複 upsert 不應新增資料列");
    assert_eq!(fetched[0].total.map(|o| o.cagr_pct), Some(dec!(99.9999)));
    assert_eq!(fetched[0].dividend_events, 7);

    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_incomplete_record_reads_back_as_none() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_incomplete_record_reads_back_as_none：無資料庫連接");
        return;
    }

    let repo = PgCagrRepository::new();
    cleanup().await;

    repo.save_batch(&[incomplete_record(FAKE_SYMBOL, CagrPeriod::Y10)])
        .await
        .expect("save_batch");

    let fetched = repo
        .fetch_by_symbol(test_date(), FAKE_SYMBOL)
        .await
        .expect("fetch_by_symbol");
    assert_eq!(fetched.len(), 1);
    let item = &fetched[0];
    assert!(!item.data_complete);
    assert!(item.base_price.is_none());
    assert!(item.end_price.is_none());
    assert!(item.years.is_none());
    assert!(item.price.is_none());
    assert!(item.total.is_none(), "資料不足時不可讀成 0，必須是 None");
    assert!(item.reinvested.is_none());
    assert!(item.shortfall_days.is_none());
    assert!(item.has_anomaly);

    cleanup().await;
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_fetch_by_symbol_sorted_by_period_length() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 test_fetch_by_symbol_sorted_by_period_length：無資料庫連接");
        return;
    }

    let repo = PgCagrRepository::new();
    cleanup().await;

    // 刻意以「字典序會排錯」的組合驗證：Y10 字典序在 Y1H 之前。
    let records = vec![
        complete_record(FAKE_SYMBOL, CagrPeriod::Y10, dec!(1)),
        complete_record(FAKE_SYMBOL, CagrPeriod::M3, dec!(2)),
        incomplete_record(FAKE_SYMBOL, CagrPeriod::Y1H),
        complete_record(FAKE_SYMBOL, CagrPeriod::Y1, dec!(3)),
    ];
    repo.save_batch(&records).await.expect("save_batch");

    let fetched = repo
        .fetch_by_symbol(test_date(), FAKE_SYMBOL)
        .await
        .expect("fetch_by_symbol");
    let periods: Vec<CagrPeriod> = fetched.iter().map(|item| item.period).collect();
    assert_eq!(
        periods,
        vec![
            CagrPeriod::M3,
            CagrPeriod::Y1,
            CagrPeriod::Y1H,
            CagrPeriod::Y10
        ],
        "應依期間長度排序，且包含 data_complete = false 的列"
    );

    cleanup().await;
}

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
