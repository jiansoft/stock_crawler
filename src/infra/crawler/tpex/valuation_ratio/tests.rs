//! 上櫃個股日本益比與股價淨值比的測試；連外的測試標 #[ignore]。

use rust_decimal_macros::dec;

use super::*;

fn fixture() -> PeQryResponse {
    let path = format!(
        "{}/src/infra/crawler/tpex/testdata/pe_qry_date_20200630.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("讀取 fixture"))
        .expect("解析 fixture")
}

/// 依欄名取本益比與淨值比；`N/A` 為沒有值。
#[test]
fn parse_fixture() {
    let ratios = parse(fixture()).expect("可解析");
    let farm = ratios
        .iter()
        .find(|ratio| ratio.stock_symbol == "1240")
        .expect("有 1240");
    assert_eq!(farm.price_earning_ratio, Some(dec!(14.25)));
    assert_eq!(farm.price_to_book_ratio, Some(dec!(1.87)));
    let loss = ratios
        .iter()
        .find(|ratio| ratio.stock_symbol == "1258")
        .expect("有 1258");
    assert_eq!(loss.price_earning_ratio, None);
    assert_eq!(loss.price_to_book_ratio, Some(dec!(0.85)));
}

/// 沒有 table 或沒有資料（非交易日）回空陣列；有資料卻缺代號欄位回錯誤。
#[test]
fn parse_empty_and_bad_layout() {
    assert!(parse(PeQryResponse::default()).expect("可解析").is_empty());
    let holiday = PeQryResponse {
        tables: vec![PeQryTable::default()],
    };
    assert!(parse(holiday).expect("可解析").is_empty());

    let renamed = PeQryResponse {
        tables: vec![PeQryTable {
            fields: vec!["代號".to_owned()],
            data: vec![vec!["6488".into()]],
        }],
    };
    assert!(parse(renamed).is_err());
}

/// 實網：取 2020-06-30 的資料，確認端點與欄位名稱仍可用。
#[tokio::test]
#[ignore = "連線外部網站"]
async fn test_visit_live() {
    dotenvy::dotenv().ok();
    let date = NaiveDate::from_ymd_opt(2020, 6, 30).expect("有效日期");
    let ratios = visit(date).await.expect("可取得");
    assert!(ratios.len() > 500, "{}", ratios.len());
    assert!(ratios.iter().any(|ratio| ratio.stock_symbol == "6488"));
}
