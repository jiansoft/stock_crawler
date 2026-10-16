//! 除權息提醒的市場清單與持股預估股利訊息測試。

use super::*;
use rust_decimal::Decimal;

#[test]
fn test_build_holding_dividend_message_groups_by_stock_and_member() {
    use crate::domain::portfolio::entity::StockOwnershipDetail;
    use chrono::{Local, NaiveDate, TimeZone};
    use rust_decimal_macros::dec;

    let today = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
    let stocks = vec![
        StockDividendInfo {
            stock_symbol: "2330".to_string(),
            name: "台積電".to_string(),
            stock_industry_id: crate::core::declare::Industry::Semiconductor.serial(),
            cash_dividend: dec!(3.5),
            stock_dividend: dec!(0.2),
            sum: dec!(3.7),
            closing_price: dec!(950),
            dividend_yield: dec!(0.39),
            cash_dividend_yield: dec!(0.37),
            is_cash_ex_dividend_on_date: true,
            is_stock_ex_dividend_on_date: false,
            quarter: String::new(),
        },
        StockDividendInfo {
            stock_symbol: "2317".to_string(),
            name: "鴻海".to_string(),
            stock_industry_id: crate::core::declare::Industry::ElectronicComponents.serial(),
            cash_dividend: dec!(5),
            stock_dividend: dec!(0.3),
            sum: dec!(5.3),
            closing_price: dec!(150),
            dividend_yield: dec!(3.53),
            cash_dividend_yield: dec!(3.33),
            is_cash_ex_dividend_on_date: true,
            is_stock_ex_dividend_on_date: true,
            quarter: String::new(),
        },
    ];
    let holdings = vec![
        StockOwnershipDetail {
            serial: 1,
            security_code: "2330".to_string(),
            member_id: 1,
            share_quantity: 1000,
            share_price_average: Decimal::ZERO,
            current_cost_per_share: dec!(600),
            holding_cost: dec!(-600000),
            is_sold: false,
            cumulate_dividends_cash: Decimal::ZERO,
            cumulate_dividends_stock: Decimal::ZERO,
            cumulate_dividends_stock_money: Decimal::ZERO,
            cumulate_dividends_total: Decimal::ZERO,
            created_time: Local.with_ymd_and_hms(2026, 3, 31, 9, 0, 0).unwrap(),
        },
        StockOwnershipDetail {
            serial: 2,
            security_code: "2330".to_string(),
            member_id: 2,
            share_quantity: 500,
            share_price_average: Decimal::ZERO,
            current_cost_per_share: dec!(500),
            holding_cost: dec!(-250000),
            is_sold: false,
            cumulate_dividends_cash: Decimal::ZERO,
            cumulate_dividends_stock: Decimal::ZERO,
            cumulate_dividends_stock_money: Decimal::ZERO,
            cumulate_dividends_total: Decimal::ZERO,
            created_time: Local.with_ymd_and_hms(2026, 3, 30, 9, 0, 0).unwrap(),
        },
        StockOwnershipDetail {
            serial: 3,
            security_code: "2317".to_string(),
            member_id: 2,
            share_quantity: 2000,
            share_price_average: Decimal::ZERO,
            current_cost_per_share: dec!(120),
            holding_cost: dec!(-240000),
            is_sold: false,
            cumulate_dividends_cash: Decimal::ZERO,
            cumulate_dividends_stock: Decimal::ZERO,
            cumulate_dividends_stock_money: Decimal::ZERO,
            cumulate_dividends_total: Decimal::ZERO,
            created_time: Local.with_ymd_and_hms(2026, 3, 20, 9, 0, 0).unwrap(),
        },
        StockOwnershipDetail {
            serial: 4,
            security_code: "2317".to_string(),
            member_id: 2,
            share_quantity: 1000,
            share_price_average: Decimal::ZERO,
            current_cost_per_share: dec!(120),
            holding_cost: dec!(-120000),
            is_sold: false,
            cumulate_dividends_cash: Decimal::ZERO,
            cumulate_dividends_stock: Decimal::ZERO,
            cumulate_dividends_stock_money: Decimal::ZERO,
            cumulate_dividends_total: Decimal::ZERO,
            created_time: Local.with_ymd_and_hms(2026, 4, 1, 9, 0, 0).unwrap(),
        },
    ];

    let msg = EventDispatcher::build_holding_dividend_message(today, &stocks, &holdings).unwrap();

    assert!(msg.contains("2330"));
    assert!(msg.contains("Eddie"));
    assert!(msg.contains("持股:1,000股"));
    assert!(msg.contains("成本:600,000元\\(600元\\)"));
    assert!(msg.contains("現金股利:3,500元"));
    assert!(msg.contains("股票股利:0元"));
    assert!(msg.contains("現金殖利率:0\\.58%"));
    assert!(msg.contains("殖利率:0\\.58%"));
    assert!(msg.contains("Unice"));
    assert!(msg.contains("持股:2,000股"));
    assert!(msg.contains("成本:240,000元\\(120元\\)"));
    assert!(msg.contains("現金股利:10,000元"));
    assert!(msg.contains("股票股利:600元"));
    assert!(msg.contains("現金殖利率:4\\.17%"));
    assert!(msg.contains("殖利率:4\\.42%"));
    assert!(!msg.contains("持股:3000股"));
}

#[test]
fn test_market_dividend_message_groups_stocks_before_etfs_and_sorts_each_group_by_yield() {
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    let today = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
    let mut stocks = vec![
        StockDividendInfo {
            stock_symbol: "0050".to_string(),
            name: "元大台灣50".to_string(),
            stock_industry_id: crate::core::declare::Industry::ExchangeTradedFund.serial(),
            cash_dividend: dec!(2),
            stock_dividend: Decimal::ZERO,
            sum: dec!(2),
            closing_price: dec!(100),
            dividend_yield: dec!(2),
            cash_dividend_yield: dec!(2),
            is_cash_ex_dividend_on_date: true,
            is_stock_ex_dividend_on_date: false,
            quarter: String::new(),
        },
        StockDividendInfo {
            stock_symbol: "2317".to_string(),
            name: "鴻海".to_string(),
            stock_industry_id: crate::core::declare::Industry::ElectronicComponents.serial(),
            cash_dividend: dec!(5),
            stock_dividend: Decimal::ZERO,
            sum: dec!(5),
            closing_price: dec!(100),
            dividend_yield: dec!(5),
            cash_dividend_yield: dec!(5),
            is_cash_ex_dividend_on_date: true,
            is_stock_ex_dividend_on_date: false,
            quarter: String::new(),
        },
        StockDividendInfo {
            stock_symbol: "00878".to_string(),
            name: "國泰永續高股息".to_string(),
            stock_industry_id: crate::core::declare::Industry::ExchangeTradedFund.serial(),
            cash_dividend: dec!(3),
            stock_dividend: Decimal::ZERO,
            sum: dec!(3),
            closing_price: dec!(100),
            dividend_yield: dec!(3),
            cash_dividend_yield: dec!(3),
            is_cash_ex_dividend_on_date: true,
            is_stock_ex_dividend_on_date: false,
            quarter: String::new(),
        },
        StockDividendInfo {
            stock_symbol: "2330".to_string(),
            name: "台積電".to_string(),
            stock_industry_id: crate::core::declare::Industry::Semiconductor.serial(),
            cash_dividend: dec!(1),
            stock_dividend: Decimal::ZERO,
            sum: dec!(1),
            closing_price: dec!(100),
            dividend_yield: dec!(1),
            cash_dividend_yield: dec!(1),
            is_cash_ex_dividend_on_date: true,
            is_stock_ex_dividend_on_date: false,
            quarter: String::new(),
        },
    ];

    EventDispatcher::sort_market_dividend_info(&mut stocks);
    let msg = EventDispatcher::build_market_dividend_message(
        today,
        "進行除權息的股票與 ETF 如下︰",
        &stocks,
    );

    let stock_section = msg.find("股票︰").unwrap();
    let etf_section = msg.find("ETF︰").unwrap();
    let hon_hai = msg.find("2317").unwrap();
    let tsmc = msg.find("2330").unwrap();
    let high_yield_etf = msg.find("00878").unwrap();
    let low_yield_etf = msg.find("0050").unwrap();

    assert!(msg.contains("2026\\-04\\-01 進行除權息的股票與 ETF 如下︰"));
    assert!(stock_section < hon_hai);
    assert!(hon_hai < tsmc);
    assert!(tsmc < etf_section);
    assert!(etf_section < high_yield_etf);
    assert!(high_yield_etf < low_yield_etf);
}

/// 一年多次配息的列在殖利率後加上年化殖利率與配息頻率；年配的列不加。
#[test]
fn market_dividend_message_shows_annualized_yield_for_frequent_payers() {
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    let etf = |symbol: &str, cash: Decimal, price: Decimal, yield_pct: Decimal, quarter: &str| {
        StockDividendInfo {
            stock_symbol: symbol.to_string(),
            name: symbol.to_string(),
            stock_industry_id: crate::core::declare::Industry::ExchangeTradedFund.serial(),
            cash_dividend: cash,
            stock_dividend: Decimal::ZERO,
            sum: cash,
            closing_price: price,
            dividend_yield: yield_pct,
            cash_dividend_yield: yield_pct,
            is_cash_ex_dividend_on_date: true,
            is_stock_ex_dividend_on_date: false,
            quarter: quarter.to_string(),
        }
    };
    let stocks = vec![
        etf("00953B", dec!(0.067), dec!(9.49), dec!(0.71), "M09"),
        etf("00878", dec!(0.4), dec!(20), dec!(2), "Q3"),
        etf("0050", dec!(1), dec!(100), dec!(1), "H1"),
        etf("2330", dec!(5), dec!(1000), dec!(0.5), ""),
    ];
    let msg = EventDispatcher::build_market_dividend_message(
        NaiveDate::from_ymd_opt(2026, 10, 6).unwrap(),
        "進行除權息的股票與 ETF 如下︰",
        &stocks,
    );

    assert!(
        msg.contains("殖利率:0\\.71% 年化殖利率:8\\.47%（月配）\n"),
        "{msg}"
    );
    assert!(msg.contains("殖利率:2% 年化殖利率:8%（季配）\n"), "{msg}");
    assert!(msg.contains("殖利率:1% 年化殖利率:2%（半年配）\n"), "{msg}");
    assert!(msg.contains("殖利率:0\\.5%\n"), "{msg}");
}

/// 持股預估訊息同樣以持股成本計算的殖利率乘上配息次數。
#[test]
fn holding_dividend_message_shows_annualized_yield_for_frequent_payers() {
    use crate::domain::portfolio::entity::StockOwnershipDetail;
    use chrono::{Local, NaiveDate, TimeZone};
    use rust_decimal_macros::dec;

    let today = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
    let stocks = vec![StockDividendInfo {
        stock_symbol: "00953B".to_string(),
        name: "群益優選非投等債".to_string(),
        stock_industry_id: crate::core::declare::Industry::ExchangeTradedFund.serial(),
        cash_dividend: dec!(0.067),
        stock_dividend: Decimal::ZERO,
        sum: dec!(0.067),
        closing_price: dec!(9.49),
        dividend_yield: dec!(0.71),
        cash_dividend_yield: dec!(0.71),
        is_cash_ex_dividend_on_date: true,
        is_stock_ex_dividend_on_date: false,
        quarter: "M09".to_string(),
    }];
    let holdings = vec![StockOwnershipDetail {
        serial: 1,
        security_code: "00953B".to_string(),
        member_id: 1,
        share_quantity: 10_000,
        share_price_average: Decimal::ZERO,
        current_cost_per_share: dec!(10),
        holding_cost: dec!(-100000),
        is_sold: false,
        cumulate_dividends_cash: Decimal::ZERO,
        cumulate_dividends_stock: Decimal::ZERO,
        cumulate_dividends_stock_money: Decimal::ZERO,
        cumulate_dividends_total: Decimal::ZERO,
        created_time: Local.with_ymd_and_hms(2026, 9, 1, 9, 0, 0).unwrap(),
    }];

    let msg = EventDispatcher::build_holding_dividend_message(today, &stocks, &holdings).unwrap();
    assert!(msg.contains("現金股利:670元"), "{msg}");
    assert!(
        msg.contains("殖利率:0\\.67% 年化殖利率:8\\.04%（月配）\n"),
        "{msg}"
    );
}
