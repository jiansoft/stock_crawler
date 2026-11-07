//! 上櫃股票的三大法人買賣超與融資融券餘額的測試；連外的測試標 #[ignore]。

use super::*;

const INSTITUTIONAL_FIXTURE: &str = include_str!("../testdata/institutional_3itrade.json");
const MARGIN_FIXTURE: &str = include_str!("../testdata/margin_balance.json");

#[test]
fn roc_date_formats_the_query_parameter() {
    assert_eq!(
        roc_date(NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()),
        "115/10/05"
    );
}

/// 5347 2026-10-05：外資合計 −2,851,652 股、投信 350,024、自營合計 −44,678，合計等於最後一欄。
#[test]
fn parse_institutional_fixture() {
    let flows = parse_institutional(serde_json::from_str(INSTITUTIONAL_FIXTURE).expect("fixture"));
    assert_eq!(flows.len(), 3);
    let vis = flows["5347"];
    assert_eq!(
        vis,
        InstitutionalFlow {
            foreign: -2_851_652,
            trust: 350_024,
            dealer: -44_678,
        }
    );
    assert_eq!(vis.total(), -2_546_306);
}

#[test]
fn parse_margin_fixture() {
    let balances = parse_margin(serde_json::from_str(MARGIN_FIXTURE).expect("fixture"));
    assert_eq!(balances.len(), 3);
    assert_eq!(
        balances["6488"],
        MarginBalance {
            margin_previous: 16_966,
            margin_today: 17_449,
            short_previous: 714,
            short_today: 715,
        }
    );
}

#[test]
fn empty_tables_yield_empty_maps() {
    assert!(parse_institutional(serde_json::from_str(r#"{"tables":[]}"#).unwrap()).is_empty());
    assert!(parse_margin(serde_json::from_str(r#"{"stat":"ok"}"#).unwrap()).is_empty());
}

#[tokio::test]
#[ignore = "live test：連線真實外部網站，需要時手動執行"]
async fn test_visit() {
    dotenvy::dotenv().ok();
    let date = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
    let flows = visit_institutional(date).await.expect("三大法人");
    let margins = visit_margin(date).await.expect("融資融券");
    dbg!(
        flows.len(),
        margins.len(),
        flows.get("6488"),
        margins.get("6488")
    );
    assert!(flows.len() > 500 && margins.len() > 500);
}
