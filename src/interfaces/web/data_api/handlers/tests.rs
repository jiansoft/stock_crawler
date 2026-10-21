//! 純函式轉換邏輯的 deterministic tests（計畫 §9 Phase 1 要求）。
//!
//! 不需要資料庫：月份編碼、期間標記對映（§3.5）與股利日期清洗都是
//! 純字串運算，行為必須與資料庫實際值域（P0-1～P0-3 實測）一一對應。

use rust_decimal::Decimal;

use super::{
    format_month, market_id_for_stats, market_id_for_stocks, parse_month, parse_optional_date,
    quarter_to_api, round2, round4, sanitize_date, valuation_band,
};

/// `YYYY-MM` ↔ `YYYYMM` 雙向轉換與各種非法輸入。
#[test]
fn month_conversion_roundtrip_and_validation() {
    assert_eq!(parse_month("2026-06"), Ok(202606));
    assert_eq!(format_month(202606), "2026-06");
    assert_eq!(parse_month("2012-01"), Ok(201201));
    // 月份超界、格式不符、缺零、混入其他字元都必須擋下。
    for invalid in [
        "2026-13", "2026-00", "2026-6", "202606", "26-06", "abcd-ef", "",
    ] {
        assert!(parse_month(invalid).is_err(), "{invalid:?} 應為非法月份");
    }
}

/// §3.5 期間標記對映：DB 空字串（年度）→ `A`，其餘原樣輸出。
#[test]
fn quarter_mapping_follows_section_3_5() {
    assert_eq!(quarter_to_api(""), "A");
    for passthrough in ["Q1", "Q2", "Q3", "Q4", "H1", "H2"] {
        assert_eq!(quarter_to_api(passthrough), passthrough);
    }
}

/// 股利日期清洗：合法日期原樣輸出；P0-3 實測的髒資料一律 `null`。
#[test]
fn dividend_date_sanitizing_drops_invalid_markers() {
    assert_eq!(sanitize_date("2026-07-15"), Some("2026-07-15".to_owned()));
    // `-`、`尚未公布`、空字串與殖利率字串都是資料庫實際存在的無效值。
    for invalid in ["-", "尚未公布", "", "1.39%", "2026/07/15", "2026-02-30"] {
        assert_eq!(sanitize_date(invalid), None, "{invalid:?} 應輸出 null");
    }
}

/// §4.4 四段估值分類的每個等號邊界必須與市場統計 SQL 完全一致。
#[test]
fn valuation_band_matches_market_breadth_boundaries() {
    let cheap = Decimal::new(100, 0);
    let fair = Decimal::new(200, 0);
    let expensive = Decimal::new(300, 0);
    assert_eq!(
        valuation_band(Decimal::new(100, 0), cheap, fair, expensive),
        "undervalued"
    );
    assert_eq!(
        valuation_band(Decimal::new(101, 0), cheap, fair, expensive),
        "fair_valued"
    );
    assert_eq!(
        valuation_band(Decimal::new(200, 0), cheap, fair, expensive),
        "fair_valued"
    );
    assert_eq!(
        valuation_band(Decimal::new(201, 0), cheap, fair, expensive),
        "overvalued"
    );
    assert_eq!(
        valuation_band(Decimal::new(300, 0), cheap, fair, expensive),
        "overvalued"
    );
    assert_eq!(
        valuation_band(Decimal::new(301, 0), cheap, fair, expensive),
        "highly_overvalued"
    );
}

/// §3.6 同一個 `market` 文字在統計表與股票表的 all 語意不同；helper
/// 回傳的 0 對股票查詢只是 SQL 展開成 `IN (2,4)` 的內部哨兵。
#[test]
fn market_mapping_follows_section_3_6() {
    for mapper in [market_id_for_stats, market_id_for_stocks] {
        assert_eq!(mapper("all"), Some(0));
        assert_eq!(mapper("twse"), Some(2));
        assert_eq!(mapper("tpex"), Some(4));
        assert_eq!(mapper("emerging"), None);
    }
}

/// 分析 endpoints 共用日期解析必須拒絕不合法日期，避免 PostgreSQL 自行
/// 寬鬆轉型造成不同 endpoint 行為不一致。
#[test]
fn analytics_date_validation_is_deterministic() {
    assert!(parse_optional_date(None).unwrap().is_none());
    assert_eq!(
        parse_optional_date(Some("2026-07-16"))
            .unwrap()
            .unwrap()
            .to_string(),
        "2026-07-16"
    );
    for invalid in ["2026-02-30", "2026/07/16", "2026-7-16", ""] {
        assert!(parse_optional_date(Some(invalid)).is_err());
    }
}

/// 遠離零捨入到小數兩位與四位。
#[test]
fn round_rounds_half_away_from_zero() {
    assert_eq!(round2(1.234_5), 1.23);
    assert_eq!(round2(-2.5_f64 / 100.0 * 3.0), -0.08);
    assert_eq!(round2(0.125), 0.13);
    assert_eq!(round4(100.0 / 101.0), 0.9901);
    assert_eq!(round4(-2.0 / 3.0), -0.6667);
}
