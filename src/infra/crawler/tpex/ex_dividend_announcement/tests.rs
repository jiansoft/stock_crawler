//! TPEx 上櫃除權除息預告表採集器的測試；連外的測試標 #[ignore]。

use rust_decimal_macros::dec;

use super::*;

/// 以真實回應 fixture 驗證解析結果。
///
/// 涵蓋：除息、除權息、純除權（現金增資，無償配股率為 0，不是股利而被丟棄）。
#[test]
fn test_parse_announcements_with_fixture() {
    // include_str! 的路徑相對於本檔案（tpex/ex_dividend_announcement/tests.rs）→ tpex/testdata/。
    const FIXTURE: &str = include_str!("../testdata/ex_dividend_prepost.json");
    let rows: Vec<TpexExRightPrepostRaw> =
        serde_json::from_str(FIXTURE).expect("fixture should parse");
    let result = parse_announcements(rows);

    assert_eq!(result.len(), 3);

    let cash_only = &result[0];
    assert_eq!(cash_only.stock_symbol, "3287");
    assert_eq!(
        cash_only.ex_date,
        chrono::NaiveDate::from_ymd_opt(2026, 8, 26).unwrap()
    );
    assert!(cash_only.is_cash);
    assert!(!cash_only.is_stock);
    assert_eq!(cash_only.cash_dividend, Some(dec!(1.5)));
    assert_eq!(cash_only.market, StockExchangeMarket::OverTheCounter);

    // 除權息：0.05999999 股/股 → 0.5999999 元。
    let both = result
        .iter()
        .find(|item| item.stock_symbol == "3128")
        .expect("3128 should exist");
    assert!(both.is_cash);
    assert!(both.is_stock);
    assert_eq!(both.cash_dividend, Some(dec!(0.4)));
    assert_eq!(both.stock_dividend(), Some(dec!(0.5999999)));

    // 純除權（現金增資）：沒有任何股利，不是股利事件。
    assert!(result.iter().all(|item| item.stock_symbol != "3234"));
}

#[test]
fn test_parse_announcements_skips_invalid_date() {
    let rows = vec![TpexExRightPrepostRaw {
        date: "115082".to_string(),
        code: "6488".to_string(),
        name: "環球晶".to_string(),
        ex_dividend: "除息".to_string(),
        stock_dividend_ratio: "0.00000000".to_string(),
        subscription_ratio: "0.00000000".to_string(),
        cash_dividend: "10.00000000".to_string(),
    }];

    assert!(parse_announcements(rows).is_empty());
}

/// 現金股利＋現增的「除權息」只算除息，否則會在只有現金股利的資料列寫入除權日。
///
/// 資料取自 2026-09-23 的 `tpex_exright_prepost`：3260 威剛。
#[test]
fn test_parse_announcements_cash_with_rights_issue_is_cash_only() {
    let rows = vec![TpexExRightPrepostRaw {
        date: "1150921".to_string(),
        code: "3260".to_string(),
        name: "威剛".to_string(),
        ex_dividend: "除權息".to_string(),
        stock_dividend_ratio: "0.00000000".to_string(),
        subscription_ratio: "0.06163263".to_string(),
        cash_dividend: "18.00773559".to_string(),
    }];

    let result = parse_announcements(rows);

    assert_eq!(result.len(), 1);
    assert!(result[0].is_cash);
    assert!(!result[0].is_stock);
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();

    match visit().await {
        Err(why) => println!("取得上櫃除權息預告表失敗: {:?}", why),
        Ok(result) => {
            println!("上櫃除權息預告 {} 筆", result.len());
            for item in result.iter().take(5) {
                println!("{:?}", item);
            }
        }
    }
}
