//! 除權除息公告共用型別的測試。

use rust_decimal_macros::dec;

use super::*;

/// 純現增除權（2890 永豐金、8042 金山電）不是股利事件。
#[test]
fn classify_ex_dividend_treats_pure_rights_issue_as_non_dividend() {
    // TWSE：無償配股率空白
    assert_eq!(
        classify_ex_dividend("權", None, Some(dec!(0.04329540))),
        (false, false)
    );
    // TPEx：無償配股率為 0
    assert_eq!(
        classify_ex_dividend("除權", Some(dec!(0)), Some(dec!(0.06723675))),
        (false, false)
    );
}

/// 現金股利＋現增（3260 威剛）只算除息。
#[test]
fn classify_ex_dividend_drops_stock_flag_for_cash_plus_rights_issue() {
    assert_eq!(
        classify_ex_dividend("除權息", Some(dec!(0)), Some(dec!(0.06163263))),
        (true, false)
    );
}

/// 真正的配股（含同時有現增的 2614）與未公布配股率的除權都要保留。
#[test]
fn classify_ex_dividend_keeps_bonus_share_events() {
    assert_eq!(
        classify_ex_dividend("權息", Some(dec!(0.08)), Some(dec!(0.38195352))),
        (true, true)
    );
    assert_eq!(
        classify_ex_dividend("權息", Some(dec!(0.08)), None),
        (true, true)
    );
    assert_eq!(classify_ex_dividend("權", None, None), (false, true));
    assert_eq!(
        classify_ex_dividend("息", None, Some(dec!(0))),
        (true, false)
    );
}
