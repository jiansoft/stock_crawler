//! 續抱持股的平均成交金額補查：回測主查詢不替續抱名單查成交金額，換股時才補。

use jiff_sqlx::ToSqlx;
use std::collections::HashMap;

use jiff::civil::Date;

use super::super::PeriodRow;
use super::super::portfolio::{fill_trade_values, missing_trade_values};
use super::super::sql::HOLD_TRADE_VALUES_SQL;
use crate::infra::database;

/// 補查續抱持股的平均成交金額（SQL 不替續抱名單查，見 [`HOLD_TRADE_VALUES_SQL`]）；`liquidity_starts` 是
/// 各換股日的成交金額起日。沒有缺漏時不查資料庫。
pub(in super::super) async fn fill_hold_trade_values(
    rows: &mut [PeriodRow],
    liquidity_starts: &HashMap<Date, Date>,
) -> Result<(), sqlx::Error> {
    let missing = missing_trade_values(rows);
    if missing.is_empty() {
        return Ok(());
    }
    let mut dates = Vec::with_capacity(missing.len());
    let mut symbols = Vec::with_capacity(missing.len());
    let mut starts = Vec::with_capacity(missing.len());
    for (date, symbol) in missing {
        if let Some(&start) = liquidity_starts.get(&date) {
            dates.push(date);
            symbols.push(symbol);
            starts.push(start);
        }
    }
    let found: Vec<(Date, String, Option<f64>)> =
        sqlx::query_as::<_, (jiff_sqlx::Date, String, Option<f64>)>(HOLD_TRADE_VALUES_SQL)
            .bind(
                dates
                    .iter()
                    .map(|value| value.to_sqlx())
                    .collect::<Vec<_>>(),
            )
            .bind(&symbols)
            .bind(
                starts
                    .iter()
                    .map(|value| value.to_sqlx())
                    .collect::<Vec<_>>(),
            )
            .fetch_all(database::get_batch_connection())
            .await
            .map(|rows| {
                rows.into_iter()
                    .map(|(c0, c1, c2)| (crate::infra::database::jiff_types::from_wire(c0), c1, c2))
                    .collect::<Vec<_>>()
            })?;
    let values: HashMap<(Date, String), f64> = found
        .into_iter()
        .filter_map(|(date, symbol, value)| value.map(|value| ((date, symbol), value)))
        .collect();
    fill_trade_values(rows, &values);
    Ok(())
}
