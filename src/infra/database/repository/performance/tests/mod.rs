//! 年化報酬倉儲的測試：共用的假資料與寫入、讀回；排行榜相關在 `ranking`。

use super::*;
use crate::domain::performance::entity::SimulationOutcome;
use crate::infra::database;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

mod ranking;

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
