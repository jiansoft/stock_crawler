//! 每週市場摘要的持股基本面警示一段：營收連續年減、三率三降、營業虧損（規則見
//! [`crate::domain::portfolio::health`]）。

use crate::domain::portfolio::health::{HoldingFundamentals, HoldingWarning};

/// 一段最多列出幾檔，其餘只計數。
const MAX_LISTED: usize = 10;

/// 持股基本面警示一段的各行（開頭為空行）；沒有警示時回空陣列。依代號排序。
pub(super) fn lines(
    fundamentals: &[HoldingFundamentals],
    stock_name: &impl Fn(&str) -> String,
) -> Vec<String> {
    let mut warned: Vec<(&HoldingFundamentals, Vec<HoldingWarning>)> = fundamentals
        .iter()
        .map(|item| (item, item.signals.warnings()))
        .filter(|(_, warnings)| !warnings.is_empty())
        .collect();
    if warned.is_empty() {
        return Vec::new();
    }
    warned.sort_by(|a, b| a.0.stock_symbol.cmp(&b.0.stock_symbol));
    let mut lines = vec![
        String::new(),
        format!("持股基本面警示（{} 檔）", warned.len()),
    ];
    for (item, warnings) in warned.iter().take(MAX_LISTED) {
        let texts: Vec<String> = warnings
            .iter()
            .map(|warning| warning_text(item, *warning))
            .collect();
        lines.push(format!(
            "• {} {}：{}",
            item.stock_symbol,
            stock_name(&item.stock_symbol),
            texts.join("、")
        ));
    }
    if warned.len() > MAX_LISTED {
        lines.push(format!("…其餘 {} 檔", warned.len() - MAX_LISTED));
    }
    lines
}

/// 單一警示的文字，附判斷依據的數字。
fn warning_text(item: &HoldingFundamentals, warning: HoldingWarning) -> String {
    match warning {
        HoldingWarning::RevenueDecline => {
            let months = item.signals.revenue_decline_months.unwrap_or_default();
            match item.revenue_yoy {
                Some(yoy) => format!(
                    "營收連 {months} 月年減（最新 {}%）",
                    yoy.round_dp(2).normalize()
                ),
                None => format!("營收連 {months} 月年減"),
            }
        }
        HoldingWarning::MarginsAllDown => "三率三降".to_owned(),
        HoldingWarning::OperatingLoss => match item.signals.operating_margin {
            Some(margin) => format!("營業虧損（營益率 {margin:.2}%）"),
            None => "營業虧損".to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
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
                "• 2330 名2330：營收連 3 月年減（最新 -12.5%）、三率三降、營業虧損（營益率 -1.46%）"
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
    }
}
