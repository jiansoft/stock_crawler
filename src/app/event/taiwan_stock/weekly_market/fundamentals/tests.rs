//! 每週市場摘要的持股基本面警示一段：營收連續年減、三率三降、營業虧損、自由現金流為負（規則見的測試。

use rust_decimal_macros::dec;

use super::*;
use crate::domain::portfolio::health::FundamentalSignals;

fn fundamentals(symbol: &str, signals: FundamentalSignals) -> HoldingFundamentals {
    HoldingFundamentals {
        stock_symbol: symbol.to_owned(),
        revenue_yoy: Some(dec!(-12.50)),
        signals,
        ..Default::default()
    }
}

/// 只列有警示的持股、依代號排序並附依據的數字；沒有警示不列這段。
#[test]
fn lists_warned_holdings_with_reasons() {
    let warned = fundamentals(
        "2330",
        FundamentalSignals {
            revenue_decline_months: Some(3),
            operating_margin: Some(-1.456),
            margin_changes: [Some(-1.0), Some(-2.0), Some(-3.0)],
            free_cash_flow_ttm: Some(-123_456),
        },
    );
    let healthy = fundamentals(
        "1101",
        FundamentalSignals {
            revenue_decline_months: Some(1),
            ..Default::default()
        },
    );
    let loss = HoldingFundamentals {
        revenue_yoy: None,
        ..fundamentals(
            "1598",
            FundamentalSignals {
                revenue_decline_months: Some(4),
                operating_margin: Some(-9.95),
                ..Default::default()
            },
        )
    };
    let name = |symbol: &str| format!("名{symbol}");
    assert_eq!(
        lines(&[warned, healthy.clone(), loss], &name),
        [
            String::new(),
            "持股基本面警示（2 檔）".to_owned(),
            "• 1598 名1598：營收連 4 月年減、營業虧損（營益率 -9.95%）".to_owned(),
            "• 2330 名2330：營收連 3 月年減（最新 -12.5%）、三率三降、營業虧損（營益率 -1.46%）、\
             自由現金流為負（近四季 -1.23 億元）"
                .to_owned(),
        ]
    );
    assert!(lines(&[healthy], &name).is_empty());
}

/// 超過上限只列前 10 檔，其餘計數。
#[test]
fn caps_the_listed_holdings() {
    let many: Vec<HoldingFundamentals> = (0..12)
        .map(|i| {
            fundamentals(
                &format!("{}", 1101 + i),
                FundamentalSignals {
                    operating_margin: Some(-1.0),
                    ..Default::default()
                },
            )
        })
        .collect();
    let lines = lines(&many, &|_: &str| String::new());
    assert_eq!(lines.len(), 2 + MAX_LISTED + 1);
    assert_eq!(lines.last().map(String::as_str), Some("…其餘 2 檔"));
    assert_eq!(
        warning_text(
            &HoldingFundamentals::default(),
            HoldingWarning::OperatingLoss
        ),
        "營業虧損"
    );
    assert_eq!(
        warning_text(
            &HoldingFundamentals::default(),
            HoldingWarning::NegativeFreeCashFlow
        ),
        "自由現金流為負"
    );
}
