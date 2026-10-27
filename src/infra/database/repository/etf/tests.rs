//! ETF 倉儲的淨值回填、基本資料、持股、績效與快照測試。

use rust_decimal_macros::dec;

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::*;
use crate::domain::etf::{
    EtfAllocationRecord, EtfHoldingRecord, EtfNavHistoryPoint, EtfNavRecord, EtfNavReturns,
    EtfPerformance, EtfPortfolio, EtfProfileRecord,
};
use crate::infra::database;

/// 測試用的假代號；寫入後一律清除。
const FAKE: &str = "79979ET";

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, d).expect("日期應合法")
}

async fn cleanup() {
    for sql in [
        "DELETE FROM etf_daily_nav WHERE stock_symbol = $1",
        "DELETE FROM etf_profile WHERE stock_symbol = $1",
        "DELETE FROM etf_holding WHERE stock_symbol = $1",
        "DELETE FROM etf_holding_history WHERE stock_symbol = $1",
        "DELETE FROM etf_allocation WHERE stock_symbol = $1",
        "DELETE FROM etf_performance WHERE stock_symbol = $1",
        "DELETE FROM etf_snapshot WHERE stock_symbol = $1",
        "DELETE FROM last_daily_quotes WHERE stock_symbol = $1",
        "DELETE FROM dividend WHERE security_code = $1",
        "DELETE FROM stocks WHERE stock_symbol = $1",
    ] {
        sqlx::query(sql)
            .bind(FAKE)
            .execute(database::get_connection())
            .await
            .expect("清除測試資料");
    }
}

/// 在加權指數日曆補上交易日（回填前一個交易日要用），回傳本測試新增的列，結束時交給 [`remove_trading_days`]。
async fn trading_days(dates: &[NaiveDate]) -> Vec<i64> {
    let mut serials = Vec::new();
    for date in dates {
        let inserted: Option<i64> = sqlx::query_scalar(
            r#"INSERT INTO "index" (category, "date") VALUES ('TAIEX', $1)
               ON CONFLICT DO NOTHING RETURNING serial"#,
        )
        .bind(date)
        .fetch_optional(database::get_connection())
        .await
        .expect("寫入交易日");
        serials.extend(inserted);
    }
    serials
}

async fn remove_trading_days(serials: &[i64]) {
    sqlx::query(r#"DELETE FROM "index" WHERE serial = ANY($1)"#)
        .bind(serials)
        .execute(database::get_connection())
        .await
        .expect("清除交易日");
}

fn nav(date: NaiveDate, units: i64, previous_nav: Option<Decimal>) -> EtfNavRecord {
    EtfNavRecord {
        date,
        stock_symbol: FAKE.to_string(),
        outstanding_units: units,
        units_change: 1_000,
        market_price: Some(dec!(20.10)),
        estimated_nav: Some(dec!(20.00)),
        premium_rate: Some(dec!(0.50)),
        previous_nav,
        source_time: None,
    }
}

mod holding;
mod nav;
mod performance;
mod snapshot;
