//! `/market/breadth` 與 `/market/index-history` 兩個市場統計 handlers的測試。

//! 市場統計 envelope 組裝的 deterministic tests。

use super::build_market_breadth_response;
use crate::interfaces::web::data_api::dto::MarketBreadth;

/// 實際資料少於 `days` 時不補洞；首筆仍須同時作為 `breadth` 與
/// `data_as_of`，空清單則由 handler 轉成 404。
#[test]
fn breadth_envelope_uses_first_available_history_row() {
    let point = MarketBreadth {
        date: "2026-07-16".to_owned(),
        market: "all".to_owned(),
        undervalued: 1,
        fair_valued: 2,
        overvalued: 3,
        highly_overvalued: 4,
        below_5_day_moving_average: 5,
        above_5_day_moving_average: 6,
        below_20_day_moving_average: 7,
        above_20_day_moving_average: 8,
        below_60_day_moving_average: 9,
        above_60_day_moving_average: 10,
        below_120_day_moving_average: 11,
        above_120_day_moving_average: 12,
        below_240_day_moving_average: 13,
        above_240_day_moving_average: 14,
        stocks_up: 15,
        stocks_down: 16,
        stocks_unchanged: 17,
        new_52_week_high: 18,
        new_52_week_low: 19,
        updated_at: None,
    };
    let response = build_market_breadth_response(vec![point.clone()]).unwrap();
    assert_eq!(response.data_as_of, "2026-07-16");
    assert_eq!(response.breadth, point);
    assert_eq!(response.breadth, response.history[0]);
    assert!(build_market_breadth_response(Vec::new()).is_none());
}
