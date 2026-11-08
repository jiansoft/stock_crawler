//! 三大財務報表（損益表、資產負債表、現金流量表）的測試。

use super::*;

#[test]
fn test_statement_period_db_codes() {
    let single = StatementPeriod::Single(Quarter::Q2);
    assert_eq!(
        (single.period_type(), single.quarter_code()),
        ("single", "Q2")
    );

    let cumulative = StatementPeriod::Cumulative(Quarter::Q4);
    assert_eq!(
        (cumulative.period_type(), cumulative.quarter_code()),
        ("cumulative", "Q4")
    );

    let annual = StatementPeriod::Annual;
    assert_eq!(
        (annual.period_type(), annual.quarter_code()),
        ("annual", "A")
    );
}
