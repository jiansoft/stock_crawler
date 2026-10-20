//! 驗證 Yahoo 股利解析的期間、日期與分組行為，無須連線外部服務（`live` 除外）。
//!
//! 子模組：`annual_merge`（真實頁面的年度合併）、`parsing`（HTML 解析）、`repeated_periods`（重複期別）、
//! `live`（連線正式站）；這裡放共用的股利列建構函式。

mod annual_merge;
mod live;
mod parsing;
mod repeated_periods;

use anyhow::anyhow;

use super::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

pub(super) fn dividend_row(
    period: &str,
    cash_dividend: &str,
    stock_dividend: &str,
    ex_dividend_date: &str,
    ex_rights_date: &str,
    payable_date1: &str,
    payable_date2: &str,
) -> String {
    format!(
        r#"
        <li>
            <div>
                <div class="Fxg(1) Fxs(1) Fxb(0%) Ta(end)">{period}</div>
                <div>unused</div>
                <div>{cash_dividend}</div>
                <div>{stock_dividend}</div>
                <div>unused</div>
                <div>unused</div>
                <div>{ex_dividend_date}</div>
                <div>{ex_rights_date}</div>
                <div>{payable_date1}</div>
                <div>{payable_date2}</div>
            </div>
        </li>
        "#
    )
}

pub(super) fn wrap_rows(rows: &[String]) -> String {
    format!(
        r#"<div id="main-2-QuoteDividend-Proxy"><ul>{}</ul></div>"#,
        rows.join("")
    )
}

pub(super) fn dividend_row_without_period(cash_dividend: &str) -> String {
    format!(
        r#"
        <li>
            <div>
                <div>missing-period</div>
                <div>unused</div>
                <div>{cash_dividend}</div>
                <div>0.3</div>
                <div>unused</div>
                <div>unused</div>
                <div>2025/07/01</div>
                <div>-</div>
                <div>2025/07/31</div>
                <div>-</div>
            </div>
        </li>
        "#
    )
}

pub(super) fn incomplete_dividend_row(period: &str) -> String {
    format!(
        r#"
        <li>
            <div>
                <div class="Fxg(1) Fxs(1) Fxb(0%) Ta(end)">{period}</div>
                <div>unused</div>
                <div>not-a-number</div>
            </div>
        </li>
        "#
    )
}

pub(super) fn periods(result: &YahooDividend, paid_year: i32) -> Vec<(String, Decimal, String)> {
    let mut rows: Vec<(String, Decimal, String)> = result
        .get_dividend_by_year(paid_year)
        .expect("發放年度應存在")
        .iter()
        .map(|d| {
            (
                d.quarter.clone(),
                d.cash_dividend,
                d.ex_dividend_date1.clone(),
            )
        })
        .collect();
    rows.sort();
    rows
}
