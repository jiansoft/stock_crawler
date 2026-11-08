//! Data API 讀取 ETF 專區資料的查詢合約：清單篩選與排序、單檔明細的測試。

use super::*;

#[test]
fn sort_whitelist_round_trips() {
    for key in [
        "trade_value",
        "total_assets",
        "premium_rate",
        "yield_12m",
        "net_flow",
        "ret_1m",
        "ret_ytd",
        "ret_1y",
        "holders",
    ] {
        assert_eq!(EtfSort::parse(key).map(EtfSort::column), Some(key));
    }
    assert_eq!(EtfSort::parse("name; DROP TABLE"), None);
}
