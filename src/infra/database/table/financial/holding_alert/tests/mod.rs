//! 持股通知查詢的測試。
//!
//! 子模組：`revenue`（月營收通知）、`quarterly`（季報通知）；這裡放共用的假代號、種資料與清理函式。

mod quarterly;
mod revenue;

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::domain::financial::entity::EpsEstimateBasis;

use super::*;

/// 測試用假代號；正式資料不會用到 7997x 這段號碼。
/// A：有完整錨點；B：什麼都沒有；C：只有淨利率可備援；D：季別不連續。
pub(super) const FAKE_SYMBOL_A: &str = "79979";
pub(super) const FAKE_SYMBOL_B: &str = "79978";
pub(super) const FAKE_SYMBOL_C: &str = "79977";
pub(super) const FAKE_SYMBOL_D: &str = "79976";

/// 測試用營收月份，固定歷史月份而非 `Local::now()`，避免與真實資料撞期。
pub(super) const TEST_DATE: i64 = 202608;
/// 錨點季底（2026 Q2）對應的營收月份。
pub(super) const ANCHOR_DATE: i64 = 202606;

/// `stock_industry.sql` 已預先建立編號 1（水泥工業）。
pub(super) const TEST_INDUSTRY: i32 = 1;

pub(super) fn fake_symbols() -> Vec<String> {
    [FAKE_SYMBOL_A, FAKE_SYMBOL_B, FAKE_SYMBOL_C, FAKE_SYMBOL_D]
        .iter()
        .map(|symbol| symbol.to_string())
        .collect()
}

/// 移除測試寫入的所有資料列；測試前後都要呼叫，避免污染資料庫。
pub(super) async fn cleanup() {
    let symbols = fake_symbols();
    let conn = database::get_connection();

    for sql in [
        r#"DELETE FROM "Revenue" WHERE stock_symbol = ANY($1)"#,
        "DELETE FROM financial_statement WHERE security_code = ANY($1)",
        "DELETE FROM stock_ownership_details WHERE security_code = ANY($1)",
        "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
    ] {
        let _ = sqlx::query(sql).bind(&symbols).execute(conn).await;
    }
}

pub(super) async fn seed_stock(symbol: &str, name: &str, industry: i32, issued_share: i64) {
    let conn = database::get_connection();

    let _ = sqlx::query(
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_industry_id, issued_share)
           VALUES ($1, $2, $1, $3, $4)"#,
    )
    .bind(symbol)
    .bind(name)
    .bind(industry)
    .bind(issued_share)
    .execute(conn)
    .await;

    let _ = sqlx::query(
        "INSERT INTO stock_ownership_details (security_code, share_quantity, is_sold)
         VALUES ($1, 1000, false)",
    )
    .bind(symbol)
    .execute(conn)
    .await;
}

pub(super) async fn seed_revenue(symbol: &str, date: i64, accumulated: Decimal, yoy: Decimal) {
    let _ = sqlx::query(
        r#"INSERT INTO "Revenue"
           ("SecurityCode", stock_symbol, "Date", "Monthly", "MonthlyAccumulated",
            "ComparedWithLastMonth", "ComparedWithLastYearSameMonth",
            "AccumulatedComparedWithLastYear")
           VALUES ($1, $1, $2, 150000, $3, 5, $4, 12.5)"#,
    )
    .bind(symbol)
    .bind(date)
    .bind(accumulated)
    .bind(yoy)
    .execute(database::get_connection())
    .await;
}

pub(super) async fn seed_financial(
    symbol: &str,
    year: i64,
    quarter: &str,
    net_income: Decimal,
    eps: Decimal,
) {
    let _ = sqlx::query(
        "INSERT INTO financial_statement
         (security_code, year, quarter, net_income, earnings_per_share)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(symbol)
    .bind(year)
    .bind(quarter)
    .bind(net_income)
    .bind(eps)
    .execute(database::get_connection())
    .await;
}

pub(super) async fn seed() {
    // A：產業 1、發行股數 1 億股，今年 Q1+Q2 連續公布且有 6 月營收 ⇒ 走錨定法。
    seed_stock(FAKE_SYMBOL_A, "測試甲", TEST_INDUSTRY, 100_000_000).await;
    seed_revenue(FAKE_SYMBOL_A, ANCHOR_DATE, dec!(600000), dec!(8)).await;
    seed_revenue(FAKE_SYMBOL_A, TEST_DATE, dec!(1000000), dec!(10)).await;
    seed_financial(FAKE_SYMBOL_A, 2026, "Q1", dec!(40), dec!(1.2)).await;
    seed_financial(FAKE_SYMBOL_A, 2026, "Q2", dec!(40), dec!(1.8)).await;
    // Q3 的季底（9 月）晚於本期（8 月），不能被當成錨點——否則會拿到還沒公布的營收期間。
    seed_financial(FAKE_SYMBOL_A, 2026, "Q3", dec!(40), dec!(99)).await;
    // 年度列與更舊的季度都不該影響結果。年度列的 quarter 在正式庫存的是**空字串**
    // （schema 註解寫 'A'，實際資料兩者都有），對它做 CAST 會丟 22P02，
    // 因此兩種寫法都放進來，確保季別編號的對照不會在任一種上炸掉。
    seed_financial(FAKE_SYMBOL_A, 2026, "", dec!(900), dec!(99)).await;
    seed_financial(FAKE_SYMBOL_A, 2026, "A", dec!(900), dec!(99)).await;
    seed_financial(FAKE_SYMBOL_A, 2025, "Q4", dec!(-500), dec!(99)).await;

    // B：stock_industry_id = 0 對不到分類，發行股數 0、淨利率 0 ⇒ 兩條路都走不通。
    seed_stock(FAKE_SYMBOL_B, "測試乙", 0, 0).await;
    seed_revenue(FAKE_SYMBOL_B, TEST_DATE, dec!(1000000), dec!(50)).await;
    seed_financial(FAKE_SYMBOL_B, 2026, "Q2", dec!(0), dec!(0)).await;

    // C：今年沒有任何季報，只有去年四季的淨利率 ⇒ 退回淨利率法。
    seed_stock(FAKE_SYMBOL_C, "測試丙", TEST_INDUSTRY, 100_000_000).await;
    seed_revenue(FAKE_SYMBOL_C, TEST_DATE, dec!(1000000), dec!(30)).await;
    for quarter in ["Q1", "Q2", "Q3", "Q4"] {
        seed_financial(FAKE_SYMBOL_C, 2025, quarter, dec!(40), dec!(1)).await;
    }

    // D：今年只有 Q2、缺 Q1，季別不連續 ⇒ 放棄錨點；淨利率也是 0 ⇒ 不推估。
    seed_stock(FAKE_SYMBOL_D, "測試丁", TEST_INDUSTRY, 100_000_000).await;
    seed_revenue(FAKE_SYMBOL_D, ANCHOR_DATE, dec!(600000), dec!(20)).await;
    seed_revenue(FAKE_SYMBOL_D, TEST_DATE, dec!(1000000), dec!(20)).await;
    seed_financial(FAKE_SYMBOL_D, 2026, "Q2", dec!(0), dec!(1.8)).await;
}
