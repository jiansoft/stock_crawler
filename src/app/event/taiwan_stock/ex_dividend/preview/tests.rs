use chrono::NaiveDate;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use sqlx::PgPool;

use super::*;
use crate::infra::database;

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
}

fn line(
    symbol: &str,
    ex_date: NaiveDate,
    cash: Decimal,
    shares: i64,
    fill: Option<&str>,
) -> UpcomingLine {
    UpcomingLine {
        dividend: UpcomingDividend {
            ex_date,
            stock_symbol: symbol.to_owned(),
            name: format!("{symbol} 名稱"),
            cash_dividend: cash,
            stock_dividend: Decimal::ZERO,
            closing_price: dec!(100),
            shares,
        },
        fill: fill.map(str::to_owned),
    }
}

/// 窗口是今天之後的第 1～3 個交易日：跳過週末與休市日，跨年也照算。
#[test]
fn preview_window_counts_trading_days_only() {
    let none = HashSet::new();
    // 週五：下週一到週三。
    assert_eq!(
        preview_window(date(2026, 12, 4), &none),
        (date(2026, 12, 7), date(2026, 12, 9))
    );
    // 週二休市：往後延一天。
    let holiday = HashSet::from([date(2026, 12, 8)]);
    assert_eq!(
        preview_window(date(2026, 12, 4), &holiday),
        (date(2026, 12, 7), date(2026, 12, 10))
    );
    // 跨年：元旦休市。
    let new_year = HashSet::from([date(2027, 1, 1)]);
    assert_eq!(
        preview_window(date(2026, 12, 30), &new_year),
        (date(2026, 12, 31), date(2027, 1, 5))
    );
}

/// 標題含窗口日期，每檔一行附填息，多檔時有合計，結尾附公告為準；全部跳脫 MarkdownV2。
#[test]
fn preview_message_lists_each_holding_with_total() {
    assert!(preview_message(date(2026, 12, 7), date(2026, 12, 9), &[]).is_none());

    let items = [
        line(
            "2330",
            date(2026, 12, 9),
            dec!(7),
            140,
            Some("近 5 年 20 次全數填息"),
        ),
        line("0056", date(2026, 12, 7), dec!(0.8), 1000, None),
    ];
    let message = preview_message(date(2026, 12, 7), date(2026, 12, 9), &items).expect("有持股");
    let lines: Vec<&str> = message.lines().collect();
    assert_eq!(
        lines,
        [
            r"持股除權息預告（12\-07～12\-09）︰",
            r"• 12\-09 2330 2330 名稱（140 股）除息 7 元、預估入帳 980 元；近 5 年 20 次全數填息",
            r"• 12\-07 0056 0056 名稱（1,000 股）除息 0\.8 元、預估入帳 800 元",
            "• 合計預估現金入帳 1,780 元（未扣二代健保與稅）",
            "僅供參考，除權息日與股利以公告為準",
        ]
    );
}

const SYMBOL: &str = "79979PV";

async fn cleanup(pool: &PgPool) {
    for sql in [
        "DELETE FROM stock_ownership_details WHERE security_code = $1",
        "DELETE FROM dividend WHERE security_code = $1",
        "DELETE FROM stocks WHERE stock_symbol = $1",
    ] {
        sqlx::query(sql)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .expect("清除測試資料");
    }
}

/// 連資料庫與 Redis：窗口內持股的除權息會預告並記下旗標，窗口外的不預告。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn send_flags_holdings_in_window_once() {
    dotenvy::dotenv().ok();
    let pool = database::get_connection();
    if sqlx::query("SELECT 1").execute(pool).await.is_err() || CLIENT.ping().await.is_err() {
        println!("跳過 send_flags_holdings_in_window_once：無資料庫或 Redis 連接");
        return;
    }
    let key = |ex_date: &str| format!("ex_dividend_preview:{SYMBOL}:{ex_date}");
    let clear_keys = || async {
        for ex_date in ["2024-12-10", "2024-12-11"] {
            let _ = CLIENT.delete(&key(ex_date)).await;
        }
    };
    cleanup(pool).await;
    clear_keys().await;
    sqlx::query(r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id, "SuspendListing") VALUES ($1, '除權息預告測試', $1, 2, 1, false)"#)
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("種入股票");
    // 12-10 在 12-05（四）的窗口 12-06～12-10 內；12-11 在窗口外。
    for (quarter, ex_date) in [("Q3", "2024-12-10"), ("Q4", "2024-12-11")] {
        sqlx::query(r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2) VALUES ($1, 2025, 2024, $2, 1.5, 0, 1.5, $3, '-', '-', '-')"#)
            .bind(SYMBOL)
            .bind(quarter)
            .bind(ex_date)
            .execute(pool)
            .await
            .expect("種入股利");
    }
    sqlx::query("INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, is_sold) VALUES (979797, $1, 2000, -1, false)")
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("種入持股");

    let result = send(date(2024, 12, 5), &HashSet::new()).await;
    let in_window = CLIENT.get_bool(&key("2024-12-10")).await;
    let outside = CLIENT.get_bool(&key("2024-12-11")).await;
    cleanup(pool).await;
    clear_keys().await;

    result.expect("預告應成功");
    assert!(in_window.unwrap_or(false), "窗口內的除息要記旗標");
    assert!(!outside.unwrap_or(false), "窗口外的不預告");
}
