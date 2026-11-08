//! 日線衍生計算的測試。

use super::*;
use crate::domain::quote::test_double::CountingQuoteRepository;
use rust_decimal_macros::dec;

fn quote(date: u32, high: Decimal, low: Decimal, pbr: Decimal) -> DomainDailyQuote {
    DomainDailyQuote {
        date: NaiveDate::from_ymd_opt(2026, 5, date).expect("日期應合法"),
        highest_price: high,
        lowest_price: low,
        price_to_book_ratio: pbr,
        ..Default::default()
    }
}

/// 淨值比是 0（ETF）時，價格新高、新低仍要記錄，淨值比欄位不動。
#[test]
fn update_qhr_fields_records_prices_without_price_to_book() {
    let mut qhr = QuoteHistoryRecord::new("0050".to_string());
    update_qhr_fields(&mut qhr, &quote(4, dec!(14.74), dec!(10.75), Decimal::ZERO));
    update_qhr_fields(&mut qhr, &quote(26, dec!(16.63), dec!(11), Decimal::ZERO));

    assert_eq!(qhr.maximum_price, dec!(16.63));
    assert_eq!(qhr.maximum_price_date_on.to_string(), "2026-05-26");
    assert_eq!(qhr.minimum_price, dec!(10.75));
    assert_eq!(qhr.minimum_price_date_on.to_string(), "2026-05-04");
    assert_eq!(qhr.maximum_price_to_book_ratio, Decimal::ZERO);
    assert_eq!(qhr.minimum_price_to_book_ratio, Decimal::ZERO);
}

/// 無成交（最低價 0）不算新低；淨值比照常更新極值。
#[test]
fn update_qhr_fields_ignores_zero_lows() {
    let mut qhr = QuoteHistoryRecord::new("2330".to_string());
    update_qhr_fields(&mut qhr, &quote(4, dec!(100), dec!(90), dec!(2.5)));
    update_qhr_fields(
        &mut qhr,
        &quote(5, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO),
    );
    update_qhr_fields(&mut qhr, &quote(6, dec!(95), dec!(91), dec!(2.1)));

    assert_eq!(qhr.minimum_price, dec!(90));
    assert_eq!(qhr.maximum_price_to_book_ratio, dec!(2.5));
    assert_eq!(qhr.minimum_price_to_book_ratio, dec!(2.1));
}

/// 代號依批次大小切開，每批都從同一個起始日重算，更新列數累加。
#[tokio::test]
async fn recalculate_moving_averages_splits_symbols_into_batches() {
    let repository = CountingQuoteRepository::default();
    let symbols: Vec<String> = (0..RECALCULATE_BATCH + 1)
        .map(|i| format!("{:04}", 1000 + i))
        .collect();
    let from = NaiveDate::from_ymd_opt(2019, 7, 1).expect("日期應合法");

    let updated = recalculate_moving_averages(&repository, &symbols, from)
        .await
        .expect("重算應成功");

    let calls = repository.recalculated();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0.len(), RECALCULATE_BATCH);
    assert_eq!(calls[1].0, vec![symbols[RECALCULATE_BATCH].clone()]);
    assert!(calls.iter().all(|(_, date)| *date == from));
    assert_eq!(repository.rebuilt_extremes(), symbols);
    // 假倉儲每檔回報一列。
    assert_eq!(updated, symbols.len() as u64);
}

/// 沒有代號就不碰資料庫。
#[tokio::test]
async fn recalculate_moving_averages_skips_an_empty_list() {
    let repository = CountingQuoteRepository::default();
    let from = NaiveDate::from_ymd_opt(2019, 7, 1).expect("日期應合法");

    let updated = recalculate_moving_averages(&repository, &[], from)
        .await
        .expect("空清單應成功");

    assert_eq!(updated, 0);
    assert!(repository.recalculated().is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn test_calculate_moving_average() {
    dotenvy::dotenv().ok();
    SHARE.load().await;
    tracing::debug!("開始 calculate_moving_average");
    let date = NaiveDate::from_ymd_opt(2026, 2, 26);
    match calculate_moving_average(date.unwrap()).await {
        Ok(_) => {
            tracing::debug!("calculate_moving_average() 完成");
        }
        Err(why) => {
            tracing::debug!("Failed to calculate_moving_average because {:?}", why);
        }
    }

    tracing::debug!("結束 calculate_moving_average");
}
