//! 日期時間工具的測試：民國日期解析、上市櫃已公告季別與季 EPS 預抓的季別。

use super::*;

/// 驗證短格式民國日期正常解析。
#[test]
fn test_parse_taiwan_date_short_parses_valid_input() {
    let date = parse_taiwan_date_short("1150409").unwrap();

    assert_eq!(date, NaiveDate::from_ymd_opt(2026, 4, 9).unwrap());
}

/// 驗證含多位元組字元的輸入回傳 None 而不是 panic。
///
/// 「台台1」恰好是 7 bytes（3+3+1），舊版只檢查 len() == 7 就做
/// `&s[0..3]` 切片——byte 3 落在字元邊界純屬僥倖，「x台台」這種
/// 內容會直接 panic。新版以 ASCII 數字檢查徹底擋掉這類輸入。
#[test]
fn test_parse_taiwan_date_short_rejects_multibyte_input() {
    assert_eq!(parse_taiwan_date_short("台台1"), None); // 7 bytes，邊界在 3/6
    assert_eq!(parse_taiwan_date_short("x台台"), None); // 7 bytes，切 0..3 會跨字元
    assert_eq!(parse_taiwan_date_short("115040９"), None); // 全形數字
}

/// 驗證長度不符或含非數字的輸入回傳 None。
#[test]
fn test_parse_taiwan_date_short_rejects_invalid_input() {
    assert_eq!(parse_taiwan_date_short(""), None);
    assert_eq!(parse_taiwan_date_short("115049"), None); // 只有 6 碼
    assert_eq!(parse_taiwan_date_short("11504091"), None); // 8 碼
    assert_eq!(parse_taiwan_date_short("115-4-9"), None); // 含分隔符
    assert_eq!(parse_taiwan_date_short("1150230"), None); // 2 月 30 日不存在
}

/// 驗證上市/上櫃最新已公告季別在各截止日邊界的切換是否正確。
#[test]
fn test_latest_published_quarter_for_listed_and_otc_by_date() {
    assert_eq!(
        latest_published_quarter_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 3, 31).unwrap()
        ),
        ReportQuarter::new(2025, Quarter::Q3)
    );
    assert_eq!(
        latest_published_quarter_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
        ),
        ReportQuarter::new(2025, Quarter::Q4)
    );
    assert_eq!(
        latest_published_quarter_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 5, 15).unwrap()
        ),
        ReportQuarter::new(2025, Quarter::Q4)
    );
    assert_eq!(
        latest_published_quarter_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 5, 16).unwrap()
        ),
        ReportQuarter::new(2026, Quarter::Q1)
    );
    assert_eq!(
        latest_published_quarter_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 8, 15).unwrap()
        ),
        ReportQuarter::new(2026, Quarter::Q2)
    );
    assert_eq!(
        latest_published_quarter_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 11, 15).unwrap()
        ),
        ReportQuarter::new(2026, Quarter::Q3)
    );
}

/// 驗證季 EPS 流程在季末後會開始預抓下一季。
#[test]
fn test_eps_report_quarter_targets_for_listed_and_otc_by_date() {
    assert_eq!(
        eps_report_quarter_targets_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2025, 12, 31).unwrap()
        ),
        vec![ReportQuarter::new(2025, Quarter::Q3)]
    );
    assert_eq!(
        eps_report_quarter_targets_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
        ),
        vec![
            ReportQuarter::new(2025, Quarter::Q3),
            ReportQuarter::new(2025, Quarter::Q4),
        ]
    );
    assert_eq!(
        eps_report_quarter_targets_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()
        ),
        vec![
            ReportQuarter::new(2025, Quarter::Q3),
            ReportQuarter::new(2025, Quarter::Q4),
        ]
    );
    assert_eq!(
        eps_report_quarter_targets_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()
        ),
        vec![
            ReportQuarter::new(2025, Quarter::Q3),
            ReportQuarter::new(2025, Quarter::Q4),
        ]
    );
    assert_eq!(
        eps_report_quarter_targets_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 3, 25).unwrap()
        ),
        vec![
            ReportQuarter::new(2025, Quarter::Q3),
            ReportQuarter::new(2025, Quarter::Q4),
        ]
    );
    assert_eq!(
        eps_report_quarter_targets_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
        ),
        vec![
            ReportQuarter::new(2025, Quarter::Q4),
            ReportQuarter::new(2026, Quarter::Q1),
        ]
    );
    assert_eq!(
        eps_report_quarter_targets_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 4, 15).unwrap()
        ),
        vec![
            ReportQuarter::new(2025, Quarter::Q4),
            ReportQuarter::new(2026, Quarter::Q1),
        ]
    );
}

/// 驗證 Yahoo 補欄位流程仍維持較保守的截止日前預抓視窗。
#[test]
fn test_backfill_report_quarter_targets_for_listed_and_otc_by_date() {
    assert_eq!(
        backfill_report_quarter_targets_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()
        ),
        vec![ReportQuarter::new(2025, Quarter::Q3)]
    );
    assert_eq!(
        backfill_report_quarter_targets_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()
        ),
        vec![
            ReportQuarter::new(2025, Quarter::Q3),
            ReportQuarter::new(2025, Quarter::Q4),
        ]
    );
    assert_eq!(
        backfill_report_quarter_targets_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
        ),
        vec![ReportQuarter::new(2025, Quarter::Q4)]
    );
    assert_eq!(
        backfill_report_quarter_targets_for_listed_and_otc_by_date(
            NaiveDate::from_ymd_opt(2026, 4, 15).unwrap()
        ),
        vec![
            ReportQuarter::new(2025, Quarter::Q4),
            ReportQuarter::new(2026, Quarter::Q1),
        ]
    );
}
