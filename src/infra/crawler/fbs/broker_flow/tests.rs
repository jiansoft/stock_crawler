//! 富邦證券（MoneyDJ）主力進出的測試；連外的測試標 #[ignore]。

use rust_decimal_macros::dec;

use super::*;

/// 2884 玉山金 2026-10-05 的頁面（已轉成 UTF-8）。
const FIXTURE_2884: &str = include_str!("../testdata/zco_2884.html");

/// 2924 宏太-KY 2026-10-05：賣超側只有一家，其餘是空白格。
const FIXTURE_2924: &str = include_str!("../testdata/zco_2924.html");

#[test]
fn build_url_points_to_broker_page() {
    assert_eq!(
        build_url("2330"),
        "https://fubon-ebrokerdj.fbs.com.tw/z/zc/zco/zco_2330.djhtm"
    );
}

/// 兩側各 15 家；主力買賣超等於頁面合計列的買超合計減賣超合計（4,415 − 8,205）。
#[test]
fn parse_reads_both_sides_and_totals() {
    let flow = parse(FIXTURE_2884, "2884").expect("解析 fixture");

    assert_eq!(flow.stock_symbol, "2884");
    assert_eq!(flow.date, NaiveDate::from_ymd_opt(2026, 10, 5).unwrap());
    assert_eq!(flow.buyers.len(), 15);
    assert_eq!(flow.sellers.len(), 15);
    assert_eq!(
        flow.buyers[0],
        BrokerNet {
            name: "凱基-台北".to_string(),
            buy: 1_769,
            sell: 401,
            net: 1_368,
            share: dec!(6.98),
        }
    );
    assert_eq!(flow.sellers[0].name, "兆豐證券");
    assert_eq!(flow.sellers[0].net, 3_011);
    assert_eq!(flow.sellers[0].share, dec!(15.37));
    assert_eq!(flow.main_net(), 4_415 - 8_205);
    assert_eq!(flow.main_share(), dec!(-19.34));
}

/// 成交清淡時一側少於 15 家：空白格不產生分點，買賣相抵為 0 的分點仍列在買超側。
#[test]
fn parse_skips_blank_cells_on_the_shorter_side() {
    let flow = parse(FIXTURE_2924, "2924").expect("解析 fixture");

    assert_eq!(flow.buyers.len(), 3);
    assert_eq!(flow.sellers.len(), 1);
    assert_eq!(flow.buyers[1].net, 0);
    assert_eq!((flow.buy_total, flow.sell_total), (1, 1));
    assert_eq!(flow.main_net(), 0);
}

/// 沒有合計列就無法得到準確的主力買賣超，整頁失敗。
#[test]
fn parse_rejects_page_without_totals() {
    let html = FIXTURE_2924.replace("合計買超張數", "合計");
    let err = parse(&html, "2924").expect_err("沒有合計列應失敗");
    assert!(err.to_string().contains("合計列"), "{err}");
}

/// 查無代號的頁面沒有表格，不能當成「沒有主力進出」。
#[test]
fn parse_rejects_page_without_table() {
    let html = "<html><body>最後更新日：2026/10/06 查無(9188)券商分點-進出明細</body></html>";
    let err = parse(html, "9188").expect_err("沒有表格應失敗");
    assert!(err.to_string().contains("表格"), "{err}");
}

/// 沒有完整日期（查無代號時只有月日）就無法判斷資料新舊，直接失敗。
#[test]
fn parse_rejects_page_without_update_date() {
    let err = parse("<html>最後更新日：10/06</html>", "9188").expect_err("沒有日期應失敗");
    assert!(err.to_string().contains("最後更新日"), "{err}");
}

/// 數字欄位格式不符時整頁失敗，不猜測。
#[test]
fn parse_rejects_malformed_numbers() {
    let html = FIXTURE_2884.replacen("1,769", "一千", 1);
    assert!(parse(&html, "2884").is_err());
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_visit() {
    dotenvy::dotenv().ok();
    let flow = visit("2330").await.expect("抓取 2330 主力進出");
    dbg!(flow.date, flow.main_net(), flow.main_share());
    assert!(!flow.buyers.is_empty());
}
