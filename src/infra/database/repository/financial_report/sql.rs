//! 三大財務報表共用的 SQL 片段：主鍵欄位、`INSERT` 前綴、`ON CONFLICT` 子句與主鍵綁定。

use sqlx::{Postgres, query_builder::Separated};

use crate::domain::financial::statement::StatementPeriod;

/// 主鍵欄位，同時是 `ON CONFLICT` 的衝突目標。
pub(super) const KEY_COLUMNS: &str = "stock_symbol, fiscal_year, period_type, quarter, source";

/// `INSERT INTO {table} AS t (主鍵, 數值欄位) `，後面接 `VALUES`。
///
/// 別名 `t` 讓 `ON CONFLICT DO UPDATE` 能引用既有列的舊值。
pub(super) fn insert_prefix(table: &str, value_columns: &[&str]) -> String {
    format!(
        "INSERT INTO {table} AS t ({KEY_COLUMNS}, {}) ",
        value_columns.join(", ")
    )
}

/// `ON CONFLICT` 子句。
///
/// 數值欄位一律覆寫、`fetched_at` 一律更新；`updated_at` 只在任一數值欄位
/// `IS DISTINCT FROM` 舊值時才更新（`NULL` 與數字之間的變動也算），
/// 讓 `updated_at` 能反映來源的事後修正，而不是每次重抓都被刷新。
pub(super) fn upsert_suffix(value_columns: &[&str]) -> String {
    let assignments = value_columns
        .iter()
        .map(|column| format!("{column} = EXCLUDED.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    let current = value_columns
        .iter()
        .map(|column| format!("t.{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    let incoming = value_columns
        .iter()
        .map(|column| format!("EXCLUDED.{column}"))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        " ON CONFLICT ({KEY_COLUMNS}) DO UPDATE SET {assignments}, fetched_at = now(), \
         updated_at = CASE WHEN ROW({current}) IS DISTINCT FROM ROW({incoming}) \
         THEN now() ELSE t.updated_at END"
    )
}

/// 綁定主鍵欄位，順序對應 [`KEY_COLUMNS`]。
pub(super) fn push_key(
    row: &mut Separated<'_, Postgres, &'static str>,
    stock_symbol: &str,
    fiscal_year: i32,
    period: StatementPeriod,
    source: &str,
) {
    row.push_bind(stock_symbol)
        .push_bind(fiscal_year)
        .push_bind(period.period_type())
        .push_bind(period.quarter_code())
        .push_bind(source);
}
