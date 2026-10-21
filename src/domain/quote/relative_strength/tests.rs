use super::*;

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("合法日期")
}

/// 百分位分數：最弱 1、最強 99、只有一檔 99。
#[test]
fn percentile_score_maps_to_1_99() {
    assert_eq!(percentile_score(0, 5), 1);
    assert_eq!(percentile_score(4, 5), 99);
    assert_eq!(percentile_score(2, 5), 50);
    assert_eq!(percentile_score(0, 1), 99);
}

/// 資料日與 20／60／120／240 日前的基準日；交易日不足為 None。
#[test]
fn base_dates_counts_back_trading_days() {
    let dates: Vec<NaiveDate> = (0..=240)
        .map(|i| ymd(2026, 10, 13) - chrono::Days::new(i))
        .collect();
    let bases = base_dates(&dates).expect("交易日足夠");
    assert_eq!(bases.end, ymd(2026, 10, 13));
    assert_eq!(bases.d20, ymd(2026, 9, 23));
    assert_eq!(bases.d60, ymd(2026, 8, 14));
    assert_eq!(bases.d120, ymd(2026, 6, 15));
    assert_eq!(bases.d240, ymd(2026, 2, 15));
    assert_eq!(bases.active_since, ymd(2026, 10, 8));
    assert!(base_dates(&dates[1..]).is_none());
    assert!(base_dates(&[]).is_none());
}

fn scored(symbol: &str, rs_score: u8) -> ScoredStock {
    ScoredStock {
        rank: 1,
        rs_score,
        weighted: 0.0,
        row: UniverseRow {
            stock_symbol: symbol.to_owned(),
            name: symbol.to_owned(),
            market_id: 2,
            industry: String::new(),
            return_20d: None,
            return_60d: 0.0,
            return_120d: 0.0,
            return_240d: 0.0,
            closing_price: None,
        },
    }
}

/// 穿越訊號：向上為 cross_above、向下為 cross_below，跨兩個門檻兩筆都列；
/// 前一天沒有分數或沒有穿越的股票不產生。
#[test]
fn crossing_signals_map_thresholds_to_indicators() {
    let date = ymd(2026, 10, 15);
    let previous: HashMap<String, u8> = [("2330", 76), ("2317", 85), ("2454", 60)]
        .into_iter()
        .map(|(symbol, score)| (symbol.to_owned(), score))
        .collect();
    let signals = crossing_signals(
        date,
        &previous,
        &[
            scored("2330", 82),
            scored("2317", 40),
            scored("2454", 70),
            scored("6669", 99),
        ],
    );
    let actual: Vec<(&str, &str, SignalKind, &str)> = signals
        .iter()
        .map(|s| {
            assert_eq!(s.date, date);
            (
                s.stock_symbol.as_str(),
                s.signal.indicator,
                s.signal.kind,
                s.signal.description.as_str(),
            )
        })
        .collect();
    assert_eq!(
        actual,
        [
            (
                "2330",
                "rs80",
                SignalKind::CrossAbove,
                "相對強度 76 → 82 分，突破 80 分（第 1 名／4 檔）"
            ),
            (
                "2317",
                "rs80",
                SignalKind::CrossBelow,
                "相對強度 85 → 40 分，跌破 80 分（第 1 名／4 檔）"
            ),
            (
                "2317",
                "rs50",
                SignalKind::CrossBelow,
                "相對強度 85 → 40 分，跌破 50 分（第 1 名／4 檔）"
            ),
        ]
    );
}

/// 只有 rs80／rs50 的站上、跌破能還原成門檻與方向。
#[test]
fn parse_crossing_maps_indicator_and_kind() {
    assert_eq!(
        parse_crossing("rs80", "cross_above"),
        Some((80, CrossDirection::Up))
    );
    assert_eq!(
        parse_crossing("rs50", "cross_below"),
        Some((50, CrossDirection::Down))
    );
    assert_eq!(parse_crossing("rs50", "new_high"), None);
    assert_eq!(parse_crossing("close_ma60", "cross_above"), None);
    assert_eq!(parse_crossing("rs80", "unknown"), None);
}

/// 穿越：同一天跨兩個門檻兩筆都列（高門檻在前）；剛好等於門檻算在門檻以上；沒穿越為空。
#[test]
fn crossings_detect_thresholds() {
    assert_eq!(crossings(79, 80), [(80, CrossDirection::Up)]);
    assert_eq!(crossings(80, 79), [(80, CrossDirection::Down)]);
    assert_eq!(
        crossings(85, 40),
        [(80, CrossDirection::Down), (50, CrossDirection::Down)]
    );
    assert_eq!(crossings(49, 50), [(50, CrossDirection::Up)]);
    assert!(crossings(60, 70).is_empty());
    assert!(crossings(80, 99).is_empty());
    assert_eq!(CrossDirection::Up.as_str(), "up");
    assert_eq!(CrossDirection::Down.as_str(), "down");
}
