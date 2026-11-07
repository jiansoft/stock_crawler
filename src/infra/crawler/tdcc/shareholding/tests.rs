//! 集保戶股權分散表的測試；連外的測試標 #[ignore]。

use rust_decimal_macros::dec;

use super::*;

const FIXTURE: &str = include_str!("../testdata/shareholding_distribution.csv");

/// 2330 2026-10-02：千張大戶 1,485 人、持股 84.77%，股東 3,010,913 人。
#[test]
fn summarize_fixture() {
    let distributions = summarize(&parse(FIXTURE));
    assert_eq!(distributions.len(), 2);
    assert_eq!(
        distributions["2330"],
        HolderDistribution {
            date: NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
            major_holders: 1_485,
            major_percent: dec!(84.77),
            total_holders: 3_010_913,
            holders_400: 582 + 339 + 224 + 1_485,
            shares_400: 283_883_740 + 235_281_628 + 200_128_136 + 21_984_287_365,
            percent_400: Some(dec!(87.5492)),
        }
    );
    assert!(distributions.contains_key("2884"));
}

/// 合計股數為 0（或負數）時沒有比例。
#[test]
fn share_percent_needs_positive_total() {
    assert_eq!(share_percent(1, 3), Some(dec!(33.3333)));
    assert_eq!(share_percent(0, 0), None);
    assert_eq!(share_percent(5, -1), None);
}

/// 每檔 17 列全部保留；第 12 級（400,001～600,000 股）與合計列的數值原樣取出。
#[test]
fn parse_keeps_every_level() {
    let levels = parse(FIXTURE);
    assert_eq!(levels.len(), 34);
    let of_2330: Vec<_> = levels.iter().filter(|l| l.stock_symbol == "2330").collect();
    assert_eq!(
        of_2330.iter().map(|l| l.level).collect::<Vec<_>>(),
        (1..=17).collect::<Vec<i16>>()
    );
    assert_eq!(
        *of_2330[11],
        HolderLevel {
            date: NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
            stock_symbol: "2330".to_string(),
            level: 12,
            holders: 582,
            shares: 283_883_740,
            percent: dec!(1.09),
        }
    );
    assert_eq!(of_2330[16].shares, 25_932_370_067);
    assert_eq!((of_2330[15].level, of_2330[15].holders), (16, 2));
}

/// 缺千張大戶或合計列的證券不列入摘要；壞列與超出範圍的分級略過。
#[test]
fn parse_skips_incomplete_symbols_and_bad_lines() {
    let body = "資料日期,證券代號,持股分級,人數,股數,占集保庫存數比例%\n\
                20261002,1234  ,15,10,2000000,50.00\n\
                20261002,5678  ,15,3,3000000,30.00\n\
                20261002,5678  ,17,900,10000000,100.00\n\
                20261002,5678  ,18,1,1,0.00\n\
                20261002,5678  ,0,1,1,0.00\n\
                20261002,      ,1,1,1,0.00\n\
                bad,line\n\
                20261002,9999  ,17,x,1,100.00\n\
                20261002,9999  ,1,1,y,100.00\n";
    let levels = parse(body);
    assert_eq!(levels.len(), 3);
    let distributions = summarize(&levels);
    assert_eq!(distributions.len(), 1);
    assert_eq!(distributions["5678"].major_percent, dec!(30.00));
    assert_eq!(distributions["5678"].total_holders, 900);
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_visit() {
    dotenvy::dotenv().ok();
    let distributions = visit().await.expect("集保股權分散表");
    dbg!(distributions.len(), distributions.get("2330"));
    assert!(distributions.len() > 2_000);
}
