//! `/market/valuation` handler：上市櫃本益比與股價淨值比中位數的走勢與歷史位置的測試。

use super::*;

fn day(date: (i32, u32, u32), pe: Option<f64>) -> MarketValuationDay {
    MarketValuationDay {
        date: NaiveDate::from_ymd_opt(date.0, date.1, date.2).expect("合法日期"),
        median_pe: pe,
        median_pb: pe.map(|value| value / 10.0),
        pe_samples: 100,
        pb_samples: 100,
    }
}

/// 回應取最新一天為 current、每月最後一個交易日為 monthly；數值四捨五入到小數兩位；沒有資料回 None。
#[test]
fn build_response_samples_month_ends() {
    let days = vec![
        day((2026, 1, 29), Some(14.0)),
        day((2026, 1, 30), Some(15.0)),
        day((2026, 2, 2), Some(16.0)),
        day((2026, 2, 27), Some(13.333)),
    ];
    let response = build_response(days.clone(), 5, "all").expect("有資料");
    assert_eq!(
        (response.from.as_str(), response.data_as_of.as_str()),
        ("2026-01-29", "2026-02-27")
    );
    assert_eq!(
        response
            .monthly
            .iter()
            .map(|point| point.date.as_str())
            .collect::<Vec<_>>(),
        ["2026-01-30", "2026-02-27"]
    );
    assert_eq!(response.current.median_pe, Some(13.33));
    let pe = response.pe.expect("有本益比");
    assert_eq!((pe.percentile, pe.min, pe.days), (25.0, 13.33, 4));
    assert_eq!(response.monthly[0].median_pb, Some(1.5));

    // 最新一天沒有本益比：pe、pb 為 null。
    let mut missing = days;
    missing.push(day((2026, 3, 2), None));
    let response = build_response(missing, 5, "all").expect("有資料");
    assert!(response.pe.is_none() && response.pb.is_none());
    assert!(build_response(Vec::new(), 5, "all").is_none());
}
