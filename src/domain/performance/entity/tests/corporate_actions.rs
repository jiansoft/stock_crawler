//! 減資、退還股款與分割的參考價換算測試。

use super::*;

fn corporate_action(action_type: CorporateActionType, share_ratio: Decimal) -> CorporateAction {
    CorporateAction {
        stock_symbol: "6550".to_string(),
        effective_date: date(2026, 9, 29),
        action_type,
        share_ratio,
        note: String::new(),
    }
}

/// 6550 2026-09-29 減資恢復買賣：前收 9.92、比例 0.51964379，參考價應約 19.09；
/// 當天成交 18.35 相對前收漲 85%，未還原參考價就會被當成異常價格整天過濾。
#[test]
fn corporate_action_reference_price_reverses_capital_reduction_ratio() {
    let action = corporate_action(
        CorporateActionType::CapitalReduction,
        Decimal::new(51964379, 8),
    );
    assert_eq!(
        action.reference_price(Decimal::new(992, 2)),
        Some(Decimal::new(1909, 2))
    );
}

/// 退還股款的比例可能大於 1（8201 前收 8.69、參考價 8.12），反除一樣回到參考價。
#[test]
fn corporate_action_reference_price_handles_cash_return_ratio_above_one() {
    let action = corporate_action(
        CorporateActionType::CapitalReduction,
        Decimal::new(869, 2) / Decimal::new(812, 2),
    );
    assert_eq!(
        action.reference_price(Decimal::new(869, 2)),
        Some(Decimal::new(812, 2))
    );
}

#[test]
fn corporate_action_reference_price_divides_by_split_ratio() {
    let action = corporate_action(CorporateActionType::Split, Decimal::from(4));
    assert_eq!(
        action.reference_price(Decimal::from(200)),
        Some(Decimal::from(50))
    );
}

#[test]
fn corporate_action_reference_price_rejects_non_positive_inputs() {
    let action = corporate_action(CorporateActionType::CapitalReduction, Decimal::ZERO);
    assert_eq!(action.reference_price(Decimal::from(10)), None);

    let action = corporate_action(CorporateActionType::CapitalReduction, Decimal::new(7, 1));
    assert_eq!(action.reference_price(Decimal::ZERO), None);
}
