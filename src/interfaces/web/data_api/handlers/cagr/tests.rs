//! 每日 CAGR endpoint 的純函式測試（M4）。
//!
//! 全部不需要資料庫：參數解析、白名單驗證與 Decimal → 字串序列化都是
//! 純運算，而這三者正是前端契約最容易在重構時被悄悄改掉的部分。

use chrono::NaiveDate;
use rust_decimal_macros::dec;

use super::{
    DomainCagrRankingItem, DomainStockCagr, cagr_market_id, cagr_period_item, cagr_ranking_item,
    decimal_ratio, decimal_shares, normalize_cagr_keyword, parse_cagr_metric, parse_cagr_period,
    representative_base_date,
};
use crate::domain::performance::entity::{CagrMetric, CagrPeriod, SimulationOutcome};
use crate::domain::performance::query::CagrSortKey;

/// 建立一筆資料齊全的領域結果。
fn complete(symbol: &str, period: CagrPeriod, base_date: NaiveDate) -> DomainStockCagr {
    DomainStockCagr {
        date: NaiveDate::from_ymd_opt(2026, 8, 6).unwrap(),
        stock_symbol: symbol.to_owned(),
        period,
        base_date: Some(base_date),
        base_price: Some(dec!(178)),
        end_price: Some(dec!(1340)),
        years: Some(dec!(10.002740)),
        price: None,
        total: Some(SimulationOutcome {
            end_shares: dec!(56.17977528),
            cash_received: dec!(6797.7528),
            end_value: dec!(82078.6517),
            total_return_pct: dec!(720.7865),
            cagr_pct: dec!(23.4318),
        }),
        reinvested: None,
        first_quote_date: NaiveDate::from_ymd_opt(2015, 3, 2),
        shortfall_days: Some(0),
        data_complete: true,
        has_anomaly: false,
        dividend_events: 38,
    }
}

/// 建立一筆資料不足的領域結果（所有數值欄位皆為 None）。
fn incomplete(symbol: &str, period: CagrPeriod) -> DomainStockCagr {
    DomainStockCagr {
        date: NaiveDate::from_ymd_opt(2026, 8, 6).unwrap(),
        stock_symbol: symbol.to_owned(),
        period,
        base_date: None,
        base_price: None,
        end_price: None,
        years: None,
        price: None,
        total: None,
        reinvested: None,
        first_quote_date: NaiveDate::from_ymd_opt(2024, 5, 1),
        shortfall_days: None,
        data_complete: false,
        has_anomaly: false,
        dividend_events: 0,
    }
}

/// 建立排行榜項目。
fn ranking_item(cagr: DomainStockCagr, rank: Option<i64>) -> DomainCagrRankingItem {
    DomainCagrRankingItem {
        cagr,
        name: "台積電".to_owned(),
        industry_id: 24,
        rank,
    }
}

/// 十年期範例使用的期初交易日。
fn base_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2016, 8, 5).unwrap()
}

/// `period` 與 `metric` 的預設值與白名單必須與前端契約一致。
#[test]
fn test_period_and_metric_defaults_and_whitelist() {
    assert_eq!(parse_cagr_period(None), Ok(CagrPeriod::Y1));
    assert_eq!(parse_cagr_metric(None), Ok(CagrMetric::Total));
    for period in CagrPeriod::ALL {
        assert_eq!(parse_cagr_period(Some(period.code())), Ok(period));
    }
    for metric in CagrMetric::ALL {
        assert_eq!(parse_cagr_metric(Some(metric.code())), Ok(metric));
    }
    // 大小寫與別名一律不接受，避免前後端對「y1」是否合法各自解讀。
    assert!(parse_cagr_period(Some("y1")).is_err());
    assert!(parse_cagr_period(Some("Y8")).is_err());
    assert!(parse_cagr_metric(Some("TOTAL")).is_err());
    assert!(parse_cagr_metric(Some("cash")).is_err());
}

/// 未指定 `sort` 時，短期間必須改用區間總報酬——年化會被放大約四倍。
#[test]
fn test_default_sort_key_follows_period_length() {
    assert_eq!(
        CagrSortKey::default_for(CagrPeriod::M3),
        CagrSortKey::TotalReturn
    );
    assert_eq!(
        CagrSortKey::default_for(CagrPeriod::M6),
        CagrSortKey::TotalReturn
    );
    for period in [CagrPeriod::Y1, CagrPeriod::Y5, CagrPeriod::Y10] {
        assert_eq!(CagrSortKey::default_for(period), CagrSortKey::Cagr);
    }
    assert_eq!(
        CagrSortKey::from_code("total_return"),
        Some(CagrSortKey::TotalReturn)
    );
    assert_eq!(CagrSortKey::from_code("cagr"), Some(CagrSortKey::Cagr));
    assert_eq!(CagrSortKey::from_code("CAGR"), None);
}

/// 長期間不得提供純價格口徑；判準取自領域層而非在 handler 硬寫。
#[test]
fn test_long_periods_reject_price_metric() {
    for period in [CagrPeriod::Y5, CagrPeriod::Y10] {
        assert!(!period.supports_price_metric(), "{period:?} 應拒絕純價格");
        assert!(period.requires_coverage_disclosure());
    }
    for period in [CagrPeriod::M3, CagrPeriod::Y1, CagrPeriod::Y3] {
        assert!(period.supports_price_metric());
        assert!(!period.requires_coverage_disclosure());
    }
}

/// `market` 白名單：`all` 代表不篩選，其餘映射到市場編號。
#[test]
fn test_market_parameter_whitelist() {
    assert_eq!(cagr_market_id("all"), Ok(None));
    assert_eq!(cagr_market_id("twse"), Ok(Some(2)));
    assert_eq!(cagr_market_id("tpex"), Ok(Some(4)));
    assert_eq!(cagr_market_id("5"), Ok(Some(5)));
    assert!(cagr_market_id("0").is_err());
    assert!(cagr_market_id("-1").is_err());
    assert!(cagr_market_id("emerging").is_err());
}

/// 關鍵字去空白後為空即視為未指定，過長則拒絕。
#[test]
fn test_keyword_normalization() {
    assert_eq!(normalize_cagr_keyword(None), Ok(None));
    assert_eq!(normalize_cagr_keyword(Some("   ")), Ok(None));
    assert_eq!(
        normalize_cagr_keyword(Some("  台積  ")),
        Ok(Some("台積".to_owned()))
    );
    assert!(normalize_cagr_keyword(Some(&"台".repeat(51))).is_err());
}

/// 金額與股數必須是固定小數位的字串——JSON number 會經過 f64 掉精度。
#[test]
fn test_decimal_serializes_as_fixed_scale_string() {
    assert_eq!(decimal_ratio(Some(dec!(178))), Some("178.0000".to_owned()));
    assert_eq!(
        decimal_ratio(Some(dec!(82078.6517))),
        Some("82078.6517".to_owned())
    );
    assert_eq!(
        decimal_ratio(Some(dec!(10.002740))),
        Some("10.0027".to_owned())
    );
    assert_eq!(
        decimal_shares(Some(dec!(56.17977528))),
        Some("56.17977528".to_owned())
    );
    assert_eq!(decimal_ratio(None), None);
    assert_eq!(decimal_shares(None), None);
}

/// 可算項目的每個欄位都要對到正確來源，且名次原樣帶出。
#[test]
fn test_ranking_item_maps_selected_metric() {
    let item = ranking_item(complete("2330", CagrPeriod::Y10, base_date()), Some(1));
    let dto = cagr_ranking_item(item, CagrMetric::Total);
    assert_eq!(dto.rank, Some(1));
    assert_eq!(dto.stock_symbol, "2330");
    assert_eq!(dto.name, "台積電");
    assert_eq!(dto.stock_industry_id, 24);
    assert_eq!(dto.base_date.as_deref(), Some("2016-08-05"));
    assert_eq!(dto.first_quote_date.as_deref(), Some("2015-03-02"));
    assert_eq!(dto.shortfall_days, Some(0));
    assert_eq!(dto.base_price.as_deref(), Some("178.0000"));
    assert_eq!(dto.end_price.as_deref(), Some("1340.0000"));
    assert_eq!(dto.end_shares.as_deref(), Some("56.17977528"));
    assert_eq!(dto.cash_received.as_deref(), Some("6797.7528"));
    assert_eq!(dto.end_value.as_deref(), Some("82078.6517"));
    assert_eq!(dto.total_return_pct.as_deref(), Some("720.7865"));
    assert_eq!(dto.cagr_pct.as_deref(), Some("23.4318"));
    assert_eq!(dto.dividend_events, 38);
    assert!(dto.data_complete);
    assert!(!dto.has_anomaly);
}

/// 未計算的口徑（此處為純價格）必須是 null，不可退回主指標的數字。
#[test]
fn test_ranking_item_missing_metric_is_null() {
    let item = ranking_item(complete("2330", CagrPeriod::Y10, base_date()), Some(1));
    let dto = cagr_ranking_item(item, CagrMetric::Price);
    assert!(dto.end_shares.is_none());
    assert!(dto.cash_received.is_none());
    assert!(dto.end_value.is_none());
    assert!(dto.total_return_pct.is_none());
    assert!(dto.cagr_pct.is_none());
    // 期初／期末價屬於該列本身，與口徑無關，仍應有值。
    assert!(dto.base_price.is_some());
}

/// 資料不足的項目要留在清單裡並回 null，而不是被省略。
#[test]
fn test_incomplete_item_keeps_row_with_null_fields() {
    let mut item = ranking_item(incomplete("6666", CagrPeriod::Y10), None);
    item.name = "測試股".to_owned();
    item.industry_id = 3;
    let dto = cagr_ranking_item(item, CagrMetric::Total);
    assert_eq!(dto.rank, None, "資料不足不得佔名次");
    assert_eq!(dto.stock_symbol, "6666");
    assert!(!dto.data_complete);
    for field in [
        &dto.base_date,
        &dto.base_price,
        &dto.end_price,
        &dto.end_shares,
        &dto.cash_received,
        &dto.end_value,
        &dto.total_return_pct,
        &dto.cagr_pct,
    ] {
        assert!(field.is_none(), "資料不足時所有數值欄位必須是 null");
    }
    assert!(dto.shortfall_days.is_none());
    // 查得到但算不出來：仍保留最早報價日，供前端說明為何算不出來。
    assert_eq!(dto.first_quote_date.as_deref(), Some("2024-05-01"));
}

/// 個股項目多一個 period、帶 years 與該期間的揭露旗標。
#[test]
fn test_period_item_carries_period_and_disclosure_flag() {
    let dto = cagr_period_item(
        complete("2330", CagrPeriod::Y10, base_date()),
        "台積電",
        24,
        CagrMetric::Total,
    );
    assert_eq!(dto.period, "Y10");
    assert_eq!(dto.years.as_deref(), Some("10.0027"));
    assert_eq!(dto.stock_industry_id, 24);
    assert!(dto.survivorship_note);

    let short = cagr_period_item(
        complete("2330", CagrPeriod::M3, base_date()),
        "台積電",
        24,
        CagrMetric::Total,
    );
    assert_eq!(short.period, "M3");
    assert!(!short.survivorship_note);
}

/// 表頭期初日取可算項目中最常見者，且不受資料不足項目干擾。
#[test]
fn test_representative_base_date_ignores_incomplete_rows() {
    let common = base_date();
    let delayed = NaiveDate::from_ymd_opt(2016, 8, 22).unwrap();
    let items = vec![
        ranking_item(complete("a", CagrPeriod::Y10, common), Some(1)),
        ranking_item(complete("b", CagrPeriod::Y10, delayed), Some(2)),
        ranking_item(complete("c", CagrPeriod::Y10, common), Some(3)),
        ranking_item(incomplete("d", CagrPeriod::Y10), None),
    ];
    assert_eq!(representative_base_date(&items), Some(common));
    assert_eq!(representative_base_date(&[]), None);
    assert_eq!(
        representative_base_date(&[ranking_item(incomplete("d", CagrPeriod::Y10), None)]),
        None
    );
}
