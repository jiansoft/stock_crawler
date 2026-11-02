//! 08:00 股利發放日通知的訊息組裝與資料庫整合測試。

use rust_decimal_macros::dec;

use super::*;

fn payout(member_id: i64, symbol: &str, day: u32, cash: Decimal) -> DividendPayout {
    DividendPayout {
        member_id,
        payable_date: NaiveDate::from_ymd_opt(2026, 11, day).expect("合法日期"),
        stock_symbol: symbol.to_owned(),
        name: format!("{symbol}名稱"),
        quarter: "Q3".to_owned(),
        ex_date: NaiveDate::from_ymd_opt(2026, 10, 20),
        shares: 1300,
        cash_dividend: dec!(3),
        stock_dividend: Decimal::ZERO,
        cash_amount: cash,
        stock_shares: 0,
        estimated: false,
    }
}

/// 今天入帳逐會員列出（含配股、推算標記），本月至今依會員加總並附合計。
#[test]
fn build_batch_dividend_message_lists_today_and_month_to_date() {
    let today = NaiveDate::from_ymd_opt(2026, 11, 20).expect("合法日期");
    let mut with_stock = payout(2, "2330", 20, dec!(1500));
    with_stock.stock_shares = 25;
    with_stock.estimated = true;
    let today_payouts = vec![payout(1, "2330", 20, dec!(3900)), with_stock];
    let mut month = today_payouts.clone();
    month.push(payout(1, "0056", 11, dec!(2872.4)));

    let msg = build_batch_dividend_message(today, &today_payouts, &month).expect("今天有入帳");

    assert!(msg.starts_with(r"2026\-11\-20 持股股利入帳如下︰"), "{msg}");
    assert!(
        msg.contains(r"[2330](https://tw\.stock\.yahoo\.com/quote/2330) 2330名稱 Eddie 持股:1,300股 現金:3,900元"),
        "{msg}"
    );
    assert!(
        msg.contains("Unice 持股:1,300股 現金:1,500元 配股:25股（以目前持股推算）"),
        "{msg}"
    );
    assert!(
        msg.contains(r"本月至今入帳現金：Eddie 6,772\.4元、Unice 1,500元，合計 8,272\.4元（未扣補充保費與匯費）"),
        "{msg}"
    );
}

/// 今天沒有持股入帳時不發。
#[test]
fn build_batch_dividend_message_returns_none_without_payouts_today() {
    let today = NaiveDate::from_ymd_opt(2026, 11, 20).expect("合法日期");
    let month = [payout(1, "0056", 11, dec!(2872.4))];

    assert!(build_batch_dividend_message(today, &[], &month).is_none());
}

#[test]
fn test_build_payable_summary_message_only_lists_items_payable_today() {
    // 2834、2838 的現金早已發放，今天只發股票股利，訊息不應再列出現金。
    let today = NaiveDate::from_ymd_opt(2026, 9, 10).unwrap();
    let stocks = vec![
        StockDividendPayableDateInfo {
            stock_symbol: "2834".to_string(),
            name: "臺企銀".to_string(),
            cash_dividend: dec!(0.3),
            stock_dividend: dec!(0.7),
            sum: dec!(1),
            payable_date1: "2026-09-02".to_string(),
            payable_date2: "2026-09-10".to_string(),
            ex_dividend_date1: "2026-08-04".to_string(),
            ex_dividend_date2: "2026-08-04".to_string(),
        },
        StockDividendPayableDateInfo {
            stock_symbol: "2838".to_string(),
            name: "聯邦銀".to_string(),
            cash_dividend: dec!(0.46),
            stock_dividend: dec!(0.6),
            sum: dec!(1.06),
            payable_date1: "2026-08-14".to_string(),
            payable_date2: "2026-09-10".to_string(),
            ex_dividend_date1: "2026-07-15".to_string(),
            ex_dividend_date2: "2026-07-15".to_string(),
        },
        StockDividendPayableDateInfo {
            stock_symbol: "2330".to_string(),
            name: "台積電".to_string(),
            cash_dividend: dec!(3),
            stock_dividend: dec!(0.2),
            sum: dec!(3.2),
            payable_date1: "2026-09-10".to_string(),
            payable_date2: "2026-09-10".to_string(),
            ex_dividend_date1: "2026-08-01".to_string(),
            ex_dividend_date2: "2026-08-01".to_string(),
        },
    ];

    let msg = build_payable_summary_message(today, &stocks).unwrap();

    assert!(msg.contains(r"2834 臺企銀 股票︰0\.7元 合計︰0\.7元"));
    assert!(msg.contains(r"2838 聯邦銀 股票︰0\.6元 合計︰0\.6元"));
    // 現金與股票同日發放時兩者都要列出，合計為兩者相加。
    assert!(msg.contains(r"2330 台積電 現金︰3元 股票︰0\.2元 合計︰3\.2元"));
    assert!(!msg.contains("現金︰0"));
}

#[test]
fn test_build_payable_summary_message_returns_none_when_nothing_payable_today() {
    let today = NaiveDate::from_ymd_opt(2026, 9, 10).unwrap();
    let stocks = vec![StockDividendPayableDateInfo {
        stock_symbol: "2834".to_string(),
        name: "臺企銀".to_string(),
        cash_dividend: dec!(0.3),
        stock_dividend: dec!(0.7),
        sum: dec!(1),
        payable_date1: "2026-09-02".to_string(),
        payable_date2: "2026-09-11".to_string(),
        ex_dividend_date1: "2026-08-04".to_string(),
        ex_dividend_date2: "2026-08-04".to_string(),
    }];

    assert!(build_payable_summary_message(today, &stocks).is_none());
}

/// 假代號與不存在的會員；固定日期 2099-01-15，結束時清除。
const SYMBOL: &str = "79979PD";
const MEMBER: i64 = 979_796;

async fn cleanup(pool: &sqlx::PgPool) {
    for sql in [
        "DELETE FROM dividend_record_detail_more WHERE stock_ownership_details_serial IN \
         (SELECT serial FROM stock_ownership_details WHERE member_id = $1)",
        "DELETE FROM dividend_record_detail WHERE stock_ownership_details_serial IN \
         (SELECT serial FROM stock_ownership_details WHERE member_id = $1)",
        "DELETE FROM stock_ownership_details WHERE member_id = $1",
    ] {
        let _ = sqlx::query(sql).bind(MEMBER).execute(pool).await;
    }
    for sql in [
        "DELETE FROM dividend WHERE security_code = $1",
        "DELETE FROM stocks WHERE stock_symbol = $1",
    ] {
        let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
    }
}

/// 種入股票、仍持有與除權息後已賣出的持股、今天與月初各一筆已領明細後跑通知流程；
/// 驗證倉儲取到的入帳（含已賣出的那筆）與月初的那筆都在本月至今內。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn notify_lists_recorded_payouts_including_sold_holdings() {
    dotenvy::dotenv().ok();
    let pool = crate::infra::database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() {
        println!("跳過 notify_lists_recorded_payouts_including_sold_holdings：無資料庫連接");
        return;
    }
    cleanup(pool).await;
    sqlx::query(r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id, "SuspendListing", last_one_eps, last_four_eps, net_asset_value_per_share, return_on_equity, weight, issued_share) VALUES ($1, '入帳測試股', $1, 2, 1, false, 1, 1, 10, 1, 0, 1000)"#)
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("種入股票");
    let holding = |shares: i64, sold: bool| async move {
        sqlx::query_scalar::<_, i64>("INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, is_sold) VALUES ($1, $2, $3, -1, $4) RETURNING serial")
            .bind(MEMBER)
            .bind(SYMBOL)
            .bind(shares)
            .bind(sold)
            .fetch_one(pool)
            .await
            .expect("種入持股")
    };
    let active = holding(1000, false).await;
    let sold = holding(500, true).await;
    let dividend = |quarter: &'static str, payable: &'static str| async move {
        sqlx::query_scalar::<_, i64>(r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2) VALUES ($1, 2099, 2098, $2, 2, 0, 2, '2098-12-10', '-', $3, '-') RETURNING serial"#)
            .bind(SYMBOL)
            .bind(quarter)
            .bind(payable)
            .fetch_one(pool)
            .await
            .expect("種入股利")
    };
    let today_dividend = dividend("Q4", "2099-01-15").await;
    let early_dividend = dividend("Q3", "2099-01-05").await;
    // 已領股利每檔持股每年一列，明細每筆股利一列。
    let detail = |holding: i64, cash: i32| async move {
        sqlx::query_scalar::<_, i64>("INSERT INTO dividend_record_detail (stock_ownership_details_serial, year, cash, total) VALUES ($1, 2099, $2, $2) RETURNING serial")
            .bind(holding)
            .bind(Decimal::from(cash))
            .fetch_one(pool)
            .await
            .expect("種入已領股利")
    };
    let active_detail = detail(active, 2300).await;
    let sold_detail = detail(sold, 1000).await;
    for (holding, detail, dividend, cash) in [
        (active, active_detail, today_dividend, 2000),
        (sold, sold_detail, today_dividend, 1000),
        (active, active_detail, early_dividend, 300),
    ] {
        sqlx::query("INSERT INTO dividend_record_detail_more (stock_ownership_details_serial, dividend_record_detail_serial, dividend_serial, cash, total) VALUES ($1, $2, $3, $4, $4)")
            .bind(holding)
            .bind(detail)
            .bind(dividend)
            .bind(Decimal::from(cash))
            .execute(pool)
            .await
            .expect("種入已領股利明細");
    }

    let today = NaiveDate::from_ymd_opt(2099, 1, 15).expect("合法日期");
    let result = notify(today).await;
    let month_start = NaiveDate::from_ymd_opt(2099, 1, 1).expect("合法日期");
    let allocations = PgPortfolioRepository::new()
        .fetch_payout_allocations(MEMBER, month_start, today, today)
        .await;
    cleanup(pool).await;

    result.expect("通知流程");
    let payouts =
        payout::split_by_payable_date(&allocations.expect("查詢入帳"), month_start, today);
    let summary: Vec<(String, i64, Decimal)> = payouts
        .iter()
        .map(|p| (p.payable_date.to_string(), p.shares, p.cash_amount))
        .collect();
    assert_eq!(
        summary,
        [
            ("2099-01-05".to_owned(), 1000, dec!(300)),
            ("2099-01-15".to_owned(), 1500, dec!(3000)),
        ],
        "已賣出的 500 股今天仍入帳"
    );
}
