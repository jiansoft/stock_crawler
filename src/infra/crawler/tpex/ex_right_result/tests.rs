//! TPEx 上櫃除權除息計算結果表採集器的測試。

use rust_decimal_macros::dec;

use super::*;

fn fixture() -> ExDailyQResponse {
    serde_json::from_str(include_str!("../testdata/ex_daily_q_202307_sample.json"))
        .expect("fixture 應為合法 JSON")
}

/// 每列取開始交易基準價（含現金增資除權），格式不符的列略過；沒有資料表回錯誤。
#[test]
fn parse_references_reads_open_base_of_every_row() {
    let references = parse_references(&fixture()).expect("解析成功");
    let rows: Vec<(&str, NaiveDate, Decimal)> = references
        .iter()
        .map(|r| (r.stock_symbol.as_str(), r.date, r.reference_price))
        .collect();
    let day = |d| NaiveDate::from_ymd_opt(2023, 7, d).expect("有效日期");
    assert_eq!(
        rows,
        [
            ("4175", day(20), dec!(115.00)),
            ("1788", day(3), dec!(140.00)),
            ("3324", day(3), dec!(290.50)),
            ("6804", day(7), dec!(75.60)),
        ]
    );

    let mut broken = fixture();
    let data = broken.tables.as_mut().expect("有資料表")[0]
        .data
        .as_mut()
        .expect("有資料");
    data[0][COL_OPEN_BASE] = "0.00".to_owned();
    data[1][COL_DATE] = "日期".to_owned();
    data[2].truncate(COL_OPEN_BASE);
    assert_eq!(parse_references(&broken).expect("解析成功").len(), 1);

    assert!(parse_references(&ExDailyQResponse { tables: None }).is_err());
}

#[test]
fn parse_results_splits_cash_and_stock_dividends() {
    let results = parse_results(&fixture()).expect("解析成功");
    // 6804 除權但無償配股為 0（現金增資），不算股利事件。
    assert_eq!(results.len(), 3);

    let both = &results[0];
    assert_eq!(both.stock_symbol, "4175");
    assert_eq!(both.ex_date, NaiveDate::from_ymd_opt(2023, 7, 20).unwrap());
    assert!(both.is_cash && both.is_stock);
    assert_eq!(both.cash_dividend, Some(dec!(2.7452)));
    assert_eq!(both.stock_dividend(), Some(dec!(0.9632195232)));
    assert_eq!(both.market, StockExchangeMarket::OverTheCounter);

    let cash_only = &results[1];
    assert_eq!(cash_only.stock_symbol, "1788");
    assert!(cash_only.is_cash && !cash_only.is_stock);
    assert_eq!(cash_only.cash_dividend, Some(dec!(7)));
    assert_eq!(cash_only.stock_dividend_ratio, None);
}

#[test]
fn missing_table_is_an_error() {
    assert!(parse_results(&ExDailyQResponse { tables: None }).is_err());
    assert!(
        parse_results(&ExDailyQResponse {
            tables: Some(vec![])
        })
        .is_err()
    );
    let empty = ExDailyQResponse {
        tables: Some(vec![ExDailyQTable { data: Some(vec![]) }]),
    };
    assert!(parse_results(&empty).expect("空表不是錯誤").is_empty());
}
