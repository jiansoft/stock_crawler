//! `/stocks/{symbol}/peers` handler：同產業個股的估值、殖利率、財務體質與營收成長比較的測試。

use rust_decimal_macros::dec;

use super::*;

fn row(symbol: &str, market_cap: Option<Decimal>, pe: Decimal, yoy: Option<Decimal>) -> PeerRow {
    PeerRow {
        stock_symbol: symbol.to_owned(),
        name: format!("{symbol} 名稱"),
        market_id: 2,
        quote_date: NaiveDate::from_ymd_opt(2026, 10, 12).expect("有效日期"),
        closing_price: dec!(100),
        market_cap,
        price_earning_ratio: pe,
        price_to_book_ratio: dec!(2),
        dividend_yield_percent: None,
        f_score: Some(6),
        gross_margin_percent: Some(dec!(30)),
        revenue_yoy_percent: yoy,
    }
}

fn metric<'a>(response: &'a PeersResponse, name: &str) -> &'a PeerMetric {
    response
        .metrics
        .iter()
        .find(|metric| metric.metric == name)
        .expect("應有此指標")
}

/// 個股欄位、指標目標值與中位數都四捨五入到小數 2 位。
#[test]
fn values_are_rounded_to_two_decimals() {
    let mut target = row("2330", Some(dec!(65868219970180.0001)), dec!(29.444), None);
    target.price_to_book_ratio = dec!(10.2399);
    target.dividend_yield_percent = Some(dec!(0.2756));
    target.gross_margin_percent = Some(dec!(67.7207));
    target.revenue_yoy_percent = Some(dec!(54.645));
    let mut other = row("2454", Some(dec!(100)), dec!(76.3849), None);
    other.price_to_book_ratio = dec!(17.4023);

    let response = build_response(
        "2330".to_owned(),
        24,
        "半導體業".to_owned(),
        20,
        vec![target, other],
    );
    let stock = response.target.as_ref().expect("有目標股");
    assert_eq!(stock.market_cap, Some(65_868_219_970_180.0));
    assert_eq!(stock.price_earning_ratio, Some(29.44));
    assert_eq!(stock.price_to_book_ratio, Some(10.24));
    assert_eq!(stock.dividend_yield_percent, Some(0.28));
    assert_eq!(stock.gross_margin_percent, Some(67.72));
    assert_eq!(
        stock.revenue_yoy_percent,
        Some(54.65),
        "四捨五入：54.645 → 54.65"
    );
    let pb = metric(&response, "price_to_book_ratio");
    assert_eq!((pb.target, pb.industry_median), (Some(10.24), Some(13.82)));
}

/// 中位數與名次用全部同業；peers 依市值由大到小只列前 limit 檔，沒有市值的排最後。
#[test]
fn build_response_ranks_target_among_all_peers() {
    let rows = vec![
        row("1101", Some(dec!(300)), dec!(10), Some(dec!(5))),
        row("1102", Some(dec!(100)), dec!(20), Some(dec!(-3))),
        row("1103", None, dec!(0), None),
        row("1104", Some(dec!(200)), dec!(30), Some(dec!(12))),
    ];
    let response = build_response("1102".to_owned(), 1, "水泥工業".to_owned(), 2, rows);

    assert_eq!(response.peer_count, 4);
    assert_eq!(response.data_as_of.as_deref(), Some("2026-10-12"));
    assert_eq!(
        response
            .peers
            .iter()
            .map(|peer| peer.stock_symbol.as_str())
            .collect::<Vec<_>>(),
        ["1101", "1104"]
    );
    let target = response.target.as_ref().expect("有目標股");
    assert_eq!(target.stock_symbol, "1102");

    let pe = metric(&response, "price_earning_ratio");
    assert_eq!(pe.better, "lower");
    assert_eq!((pe.target, pe.industry_median), (Some(20.0), Some(20.0)));
    assert_eq!((pe.rank, pe.samples), (Some(2), 3));

    let cap = metric(&response, "market_cap");
    assert_eq!((cap.rank, cap.samples), (Some(3), 3));
    let yoy = metric(&response, "revenue_yoy_percent");
    assert_eq!((yoy.rank, yoy.industry_median), (Some(3), Some(5.0)));
    let f_score = metric(&response, "f_score");
    assert_eq!((f_score.rank, f_score.samples), (Some(1), 4));
}

/// 目標股近期沒有報價：target 為 null、名次為 null，中位數照算；虧損股本益比輸出 null。
#[test]
fn build_response_without_target_quote() {
    let rows = vec![row("1101", Some(dec!(300)), dec!(0), None)];
    let response = build_response("0050".to_owned(), 0, String::new(), 20, rows);
    assert!(response.target.is_none());
    assert!(response.metrics.iter().all(|metric| metric.rank.is_none()));
    assert_eq!(response.peers[0].price_earning_ratio, None);
    assert_eq!(
        metric(&response, "price_to_book_ratio").industry_median,
        Some(2.0)
    );
}

/// 營收最舊可接受月份為今天往前 3 個月（跨年）。
#[test]
fn revenue_month_floor_goes_back_three_months() {
    let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).expect("有效日期");
    assert_eq!(revenue_month_floor(day(2026, 10, 12)), 202607);
    assert_eq!(revenue_month_floor(day(2026, 2, 28)), 202511);
}
