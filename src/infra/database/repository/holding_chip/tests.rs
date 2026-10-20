use chrono::NaiveDate;
use rust_decimal_macros::dec;

use super::*;
use crate::domain::portfolio::chip::ChipReason;

/// 測試用的假代號：只有數字、不是 00 開頭，才會被當成普通股。
const SYMBOLS: [&str; 4] = ["79971", "79972", "79973", "79974"];

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(1991, 4, d).expect("固定日期合法")
}

async fn cleanup() {
    let pool = database::get_connection();
    for sql in [
        "DELETE FROM stock_ownership_details WHERE security_code = ANY($1)",
        "DELETE FROM sell_history WHERE security_code = ANY($1)",
        "DELETE FROM chip_daily WHERE stock_symbol = ANY($1)",
        r#"DELETE FROM "DailyQuotes" WHERE stock_symbol = ANY($1)"#,
        "DELETE FROM broker_flow WHERE stock_symbol = ANY($1)",
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql)
            .bind(SYMBOLS.map(str::to_owned).to_vec())
            .execute(pool)
            .await;
    }
}

/// 交易日換算、當日持股推算（未賣、賣出日之後、當天才買、找不到賣出紀錄）與籌碼欄位對應。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn holding_chips_follow_holdings_on_each_day() {
    dotenvy::dotenv().ok();
    if database::ping().await.is_err() {
        println!("跳過 holding_chips_follow_holdings_on_each_day：無資料庫連接");
        return;
    }
    cleanup().await;
    let pool = database::get_connection();
    sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
           VALUES ('79971', '測試甲', '79971', 2, 1)"#,
    )
    .execute(pool)
    .await
    .expect("種入股票");
    // 79971 一直持有；79972 04-10 賣出；79973 04-10 才買；79974 已賣出但沒有賣出紀錄。
    for (symbol, bought, sold) in [
        ("79971", day(1), false),
        ("79972", day(1), true),
        ("79973", day(10), false),
        ("79974", day(1), true),
    ] {
        sqlx::query(
            "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, is_sold, date)
             VALUES (9, $1, 1000, -10000, $2, $3)",
        )
        .bind(symbol)
        .bind(sold)
        .bind(bought)
        .execute(pool)
        .await
        .expect("種入持股");
    }
    sqlx::query(
        "INSERT INTO sell_history (member_id, security_code, date, amount, create_time, is_reverted)
         VALUES (9, '79972', $1, 100, now(), false), (9, '79972', $2, 100, now(), true)",
    )
    .bind(day(10))
    .bind(day(9))
    .execute(pool)
    .await
    .expect("種入賣出紀錄（含一筆已撤銷）");
    for (date, symbol, foreign, streak) in [
        (day(8), "79971", 1_000_i64, 1_i16),
        (day(9), "79971", -3_000_000, -5),
        (day(9), "79972", 2_000, 1),
        (day(10), "79971", 0, 0),
    ] {
        sqlx::query(
            "INSERT INTO chip_daily (date, stock_symbol, foreign_net, trust_net, dealer_net,
                                     margin_previous, margin_balance, short_previous, short_balance,
                                     foreign_streak, trust_streak)
             VALUES ($1, $2, $3, 0, 0, 100, 100, 0, 0, $4, 0)",
        )
        .bind(date)
        .bind(symbol)
        .bind(foreign)
        .bind(streak)
        .execute(pool)
        .await
        .expect("種入籌碼");
    }
    sqlx::query(
        r#"INSERT INTO "DailyQuotes" ("Date", stock_symbol, "ClosingPrice", "TradingVolume")
           VALUES ($1, '79971', 25.5, 10000000)"#,
    )
    .bind(day(9))
    .execute(pool)
    .await
    .expect("種入日線");
    sqlx::query(
        r#"INSERT INTO broker_flow ("date", stock_symbol, buy_total, sell_total, main_share, buyers, sellers)
           VALUES ($1, '79971', 100, 300, -25.5, '[{"name":"凱基-台北","buy":150,"sell":30,"net":120,"share":"6.98"}]', '[]')"#,
    )
    .bind(day(9))
    .execute(pool)
    .await
    .expect("種入主力進出");

    let repo = PgPortfolioRepository::new();
    let resolved = repo
        .resolve_chip_day(Some(day(9)))
        .await
        .expect("換算交易日")
        .expect("有資料");
    assert_eq!(
        (resolved.date, resolved.previous, resolved.next),
        (day(9), Some(day(8)), Some(day(10)))
    );
    let fake = |chips: Vec<HoldingChip>| -> Vec<HoldingChip> {
        chips
            .into_iter()
            .filter(|chip| SYMBOLS.contains(&chip.stock_symbol.as_str()))
            .collect()
    };

    let chips = fake(
        repo.fetch_holding_chips(day(9))
            .await
            .expect("查詢持股籌碼"),
    );
    let symbols: Vec<&str> = chips
        .iter()
        .map(|chip| chip.stock_symbol.as_str())
        .collect();
    assert_eq!(symbols, ["79971", "79972"]);
    let first = &chips[0];
    assert_eq!(first.name, "測試甲");
    assert_eq!(first.closing_price, Some(dec!(25.5)));
    assert_eq!(first.volume, Some(10_000_000));
    assert_eq!(first.foreign_net, Some(-3_000_000));
    assert_eq!(first.foreign_streak, Some(-5));
    let broker = first.broker.as_ref().expect("有主力進出");
    assert_eq!((broker.main_net(), broker.main_share), (-200, dec!(-25.5)));
    assert_eq!(broker.buyers[0].share, dec!(6.98));
    assert!(broker.sellers.is_empty());
    assert_eq!(
        first.reasons(),
        [
            ChipReason::ForeignHeavy,
            ChipReason::ForeignStreak,
            ChipReason::Broker
        ]
    );
    // 79972 沒有日線與主力：成交量與主力為 None。
    assert!(chips[1].volume.is_none() && chips[1].broker.is_none());

    // 04-10：79972 已賣出、79973 當天買進。
    let chips = fake(
        repo.fetch_holding_chips(day(10))
            .await
            .expect("查詢持股籌碼"),
    );
    let symbols: Vec<&str> = chips
        .iter()
        .map(|chip| chip.stock_symbol.as_str())
        .collect();
    assert_eq!(symbols, ["79971", "79973"]);
    assert!(chips[1].foreign_net.is_none(), "79973 當天沒有籌碼列");

    cleanup().await;
}
