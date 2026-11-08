//! TWSE 上市除權除息計算結果表採集器的測試。

use rust_decimal_macros::dec;

use super::*;

fn fixture() -> Twt49uResponse {
    serde_json::from_str(include_str!("../testdata/twt49u_2025_sample.json"))
        .expect("fixture 應為合法 JSON")
}

/// 每列取開盤競價基準（權息依升降單位進位，現金增資除權為前收），格式不符的列略過。
#[test]
fn parse_references_reads_open_base_of_every_row() {
    let references = parse_references(&fixture()).expect("解析成功");
    let rows: Vec<(&str, NaiveDate, rust_decimal::Decimal)> = references
        .iter()
        .map(|r| (r.stock_symbol.as_str(), r.date, r.reference_price))
        .collect();
    let day = |m, d| NaiveDate::from_ymd_opt(2025, m, d).expect("有效日期");
    assert_eq!(
        rows,
        [
            ("00730", day(1, 17), dec!(22.90)),
            ("00730", day(2, 19), dec!(23.56)),
            ("00730", day(3, 18), dec!(22.88)),
            ("6658", day(1, 6), dec!(76.80)),
            ("1605", day(6, 4), dec!(20.40)),
        ]
    );

    let mut broken = fixture();
    let data = broken.data.as_mut().expect("有資料");
    data[0][COL_OPEN_BASE] = "--".to_owned();
    data[1][COL_DATE] = "日期".to_owned();
    data[2].truncate(COL_OPEN_BASE);
    assert_eq!(parse_references(&broken).expect("解析成功").len(), 2);

    let empty = Twt49uResponse {
        stat: Some("很抱歉，沒有符合條件的資料!".to_owned()),
        data: None,
    };
    assert!(parse_references(&empty).expect("查無資料").is_empty());
    let failed = Twt49uResponse {
        stat: Some("查詢日期大於今日".to_owned()),
        data: None,
    };
    assert!(parse_references(&failed).is_err());
}

#[test]
fn parse_results_maps_cash_only_events_with_amounts() {
    let results = parse_results(&fixture()).expect("解析成功");
    assert_eq!(results.len(), 5);

    let first = &results[0];
    assert_eq!(first.stock_symbol, "00730");
    assert_eq!(first.ex_date, NaiveDate::from_ymd_opt(2025, 1, 17).unwrap());
    assert!(first.is_cash && !first.is_stock);
    assert_eq!(first.cash_dividend, Some(dec!(0.086)));
    assert_eq!(first.market, StockExchangeMarket::Listed);
}

/// 「權」與「權息」拆不出現金與股票股利，金額留 None，只保留類別。
#[test]
fn rights_events_keep_the_kind_but_no_amounts() {
    let results = parse_results(&fixture()).expect("解析成功");
    let rights = results
        .iter()
        .find(|r| r.stock_symbol == "6658")
        .expect("6658");
    assert!(!rights.is_cash && rights.is_stock);
    assert_eq!(rights.cash_dividend, None);

    let both = results
        .iter()
        .find(|r| r.stock_symbol == "1605")
        .expect("1605");
    assert!(both.is_cash && both.is_stock);
    assert_eq!(both.cash_dividend, None);
}

#[test]
fn empty_result_and_abnormal_status() {
    let empty = Twt49uResponse {
        stat: Some("很抱歉，沒有符合條件的資料!".to_string()),
        data: None,
    };
    assert!(parse_results(&empty).expect("查無資料不是錯誤").is_empty());

    let broken = Twt49uResponse {
        stat: Some("查詢日期大於今日，請重新查詢!".to_string()),
        data: None,
    };
    assert!(parse_results(&broken).is_err());
    assert!(
        parse_results(&Twt49uResponse {
            stat: None,
            data: None
        })
        .is_err()
    );
}

#[test]
fn parse_roc_full_date_handles_the_minguo_format() {
    assert_eq!(
        parse_roc_full_date("114年01月02日"),
        NaiveDate::from_ymd_opt(2025, 1, 2)
    );
    assert_eq!(parse_roc_full_date("2025年01月02日"), None);
    assert_eq!(parse_roc_full_date("114年13月02日"), None);
    assert_eq!(parse_roc_full_date("--"), None);
}
