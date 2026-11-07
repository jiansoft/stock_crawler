//! 上市股票的三大法人買賣超與融資融券餘額的測試；連外的測試標 #[ignore]。

use super::*;

const T86_FIXTURE: &str = include_str!("../testdata/institutional_t86.json");
const MARGIN_FIXTURE: &str = include_str!("../testdata/margin_mi_margn.json");

/// 2330 2026-10-05：外資 9,770,520 股、投信 589,108、自營 491,655，合計等於 T86 的三大法人欄。
#[test]
fn parse_institutional_fixture() {
    let flows = parse_institutional(serde_json::from_str(T86_FIXTURE).expect("fixture"));
    assert_eq!(flows.len(), 3);
    let tsmc = flows["2330"];
    assert_eq!(
        tsmc,
        InstitutionalFlow {
            foreign: 9_770_520,
            trust: 589_108,
            dealer: 491_655,
        }
    );
    assert_eq!(tsmc.total(), 10_851_283);
}

/// 只讀「融資融券彙總」那張表，信用交易統計表不會混進來。
#[test]
fn parse_margin_fixture() {
    let balances = parse_margin(serde_json::from_str(MARGIN_FIXTURE).expect("fixture"));
    assert_eq!(balances.len(), 3);
    assert_eq!(
        balances["2330"],
        MarginBalance {
            margin_previous: 30_939,
            margin_today: 30_135,
            short_previous: 18,
            short_today: 46,
        }
    );
}

/// 休市或尚未公布（stat 不是 OK）時是空集合。
#[test]
fn closed_days_yield_empty_maps() {
    let closed = r#"{"stat":"很抱歉，沒有符合條件的資料!"}"#;
    assert!(parse_institutional(serde_json::from_str(closed).unwrap()).is_empty());
    assert!(parse_margin(serde_json::from_str(closed).unwrap()).is_empty());
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
        flows.get("2330"),
        margins.get("2330")
    );
    assert!(flows.len() > 1_000 && margins.len() > 1_000);
}
