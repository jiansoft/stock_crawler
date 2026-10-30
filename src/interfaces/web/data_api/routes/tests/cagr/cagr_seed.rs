//! `cagr_endpoints_db_semantics` 的測試資料寫入與清理。

use chrono::NaiveDate;
use rust_decimal_macros::dec;

use crate::domain::performance::{
    CagrPeriod, CagrRepository, StockCagr, entity::SimulationOutcome,
};
use crate::infra::database;
use crate::infra::database::repository::performance::PgCagrRepository;

/// 名次第一的假代號，同時用於個股端點與關鍵字篩選。
pub(super) const TOP_SYMBOL: &str = "79979E1";
const SECOND_SYMBOL: &str = "79979E2";
const INCOMPLETE_SYMBOL: &str = "79979E3";
/// 水泥工業，`stock_industry.sql` 已預先建立。
pub(super) const INDUSTRY: i32 = 1;
/// 上市（twse）；`cagr_market_id` 把 "twse" 對應到 2。
const MARKET: i32 = 2;

fn symbols() -> Vec<String> {
    vec![
        TOP_SYMBOL.to_string(),
        SECOND_SYMBOL.to_string(),
        INCOMPLETE_SYMBOL.to_string(),
    ]
}

fn base_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(1990, 1, 2).expect("測試日期應合法")
}

fn record(symbol: &str, period: CagrPeriod, cagr: rust_decimal::Decimal) -> StockCagr {
    let outcome = SimulationOutcome {
        end_shares: dec!(100.5),
        cash_received: dec!(250.0),
        end_value: dec!(12000.0),
        total_return_pct: dec!(20.0),
        cagr_pct: cagr,
    };
    StockCagr {
        date: base_date(),
        stock_symbol: symbol.to_string(),
        period,
        base_date: NaiveDate::from_ymd_opt(1989, 1, 3),
        base_price: Some(dec!(100.0)),
        end_price: Some(dec!(119.4)),
        years: Some(dec!(1.0)),
        // 與計算層一致：長期間不提供純價格口徑。
        price: period.supports_price_metric().then_some(outcome),
        total: Some(outcome),
        reinvested: Some(outcome),
        first_quote_date: NaiveDate::from_ymd_opt(1988, 1, 4),
        shortfall_days: Some(0),
        data_complete: true,
        has_anomaly: false,
        dividend_events: 2,
    }
}

fn incomplete(symbol: &str, period: CagrPeriod) -> StockCagr {
    StockCagr {
        date: base_date(),
        stock_symbol: symbol.to_string(),
        period,
        base_date: None,
        base_price: None,
        end_price: None,
        years: None,
        price: None,
        total: None,
        reinvested: None,
        first_quote_date: NaiveDate::from_ymd_opt(1989, 6, 1),
        shortfall_days: None,
        data_complete: false,
        has_anomaly: true,
        dividend_events: 0,
    }
}

pub(super) async fn seed() {
    for (index, symbol) in symbols().iter().enumerate() {
        let _ = sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_industry_id,
                                   stock_exchange_market_id, "SuspendListing")
               VALUES ($1, $2, $1, $3, $4, false)
               ON CONFLICT (stock_symbol) DO UPDATE
                   SET "Name" = excluded."Name",
                       stock_industry_id = excluded.stock_industry_id,
                       stock_exchange_market_id = excluded.stock_exchange_market_id"#,
        )
        .bind(symbol)
        .bind(format!("測試股{}", index + 1))
        .bind(INDUSTRY)
        .bind(MARKET)
        .execute(database::get_connection())
        .await;
    }

    // 名次第一的個股寫滿八個期間，供個股端點驗證排序；
    // 另外兩檔只寫 Y1，構成「可算 2 檔 + 資料不足 1 檔」的母體。
    let mut records: Vec<StockCagr> = CagrPeriod::ALL
        .into_iter()
        .map(|period| record(TOP_SYMBOL, period, dec!(30)))
        .collect();
    records.push(record(SECOND_SYMBOL, CagrPeriod::Y1, dec!(10)));
    records.push(incomplete(INCOMPLETE_SYMBOL, CagrPeriod::Y1));

    PgCagrRepository::new()
        .save_batch(&records)
        .await
        .expect("寫入測試用 CAGR 結果");
}

pub(super) async fn cleanup() {
    let _ = sqlx::query("DELETE FROM stock_cagr WHERE stock_symbol = ANY($1)")
        .bind(symbols())
        .execute(database::get_connection())
        .await;
    let _ = sqlx::query("DELETE FROM stocks WHERE stock_symbol = ANY($1)")
        .bind(symbols())
        .execute(database::get_connection())
        .await;
}
