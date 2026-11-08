//! `/stocks/{symbol}/technicals` handler：均線、乖離率、RSI、KD、MACD 與技術訊號的測試。

use chrono::NaiveDate;

use super::*;

fn bars(count: usize) -> Vec<PriceBar> {
    (0..count)
        .map(|i| {
            let close = 100.0 + i as f64;
            PriceBar {
                date: NaiveDate::from_ymd_opt(2026, 1, 1).expect("有效日期")
                    + chrono::Days::new(i as u64),
                close,
                high: close + 1.0,
                low: close - 1.0,
            }
        })
        .collect()
}

/// 有成交量時回傳區間突破狀態：整理幅度超過 20% 時即使帶量突破也不成立、不出訊號；
/// 沒有成交量時狀態為 null。
#[test]
fn build_response_reports_range_breakout_state() {
    let mut prices = bars(31);
    if let Some(last) = prices.last_mut() {
        last.close = 131.5;
        last.high = 132.0;
    }
    let volumes: Vec<VolumeBar> = prices
        .iter()
        .enumerate()
        .map(|(i, bar)| VolumeBar {
            date: bar.date,
            volume: if i == 30 { 2000.0 } else { 1000.0 },
        })
        .collect();

    let response = build_response("2330".to_owned(), 1, &prices, &volumes, &[]);

    assert_eq!(
        response.range_breakout,
        Some(RangeBreakoutItem {
            pivot_price: 130.0,
            base_low: 99.0,
            base_width_percent: 31.31,
            average_volume: 1000.0,
            volume_ratio: 2.0,
            distance_percent: 1.15,
            breaks_out: false,
        })
    );
    assert!(
        response
            .signals
            .iter()
            .all(|signal| signal.indicator != "range_breakout")
    );
    let without_volumes = build_response("2330".to_owned(), 1, &prices, &[], &[]);
    assert_eq!(without_volumes.range_breakout, None);
}

/// 只回傳最後 days 筆，資料日為最新一筆；持續上漲時 K 值進入超買。
#[test]
fn build_response_keeps_last_days_and_latest_signals() {
    let response = build_response("2330".to_owned(), 3, &bars(30), &[], &[]);
    assert_eq!(response.points.len(), 3);
    assert_eq!(response.data_as_of.as_deref(), Some("2026-01-30"));
    assert_eq!(response.points[0].date, "2026-01-28");
    assert!(response.adjusted);
    let last = response.points.last().expect("有最新一筆");
    assert_eq!(last.close, 129.0);
    assert_eq!(last.ma5, Some(127.0));
    assert_eq!(last.bias5, Some(1.57));
    assert!(last.ma60.is_none() && last.bias240.is_none());
    assert_eq!(last.rsi14, Some(100.0));
    assert!(last.macd_dif.is_some() && last.macd_signal.is_none());
    assert!(
        response
            .signals
            .iter()
            .any(|signal| signal.indicator == "kd" && signal.kind == "overbought")
    );
}

/// 持續上漲滿 200 個交易日後，最新一日的最高價也是 52 週新高。
#[test]
fn build_response_includes_fifty_two_week_signals() {
    let response = build_response("2330".to_owned(), 1, &bars(250), &[], &[]);
    assert!(
        response
            .signals
            .iter()
            .any(|signal| signal.indicator == "52_week" && signal.kind == "new_high"),
        "{:?}",
        response.signals
    );
}

/// 除權息事件之前的價格依參考價還原，最新一日維持實際價格。
#[test]
fn build_response_adjusts_prices_before_ex_dividend() {
    let raw = bars(30);
    let ex_date = raw[28].date;
    let dividend = PriceAdjustment {
        date: ex_date,
        cash_dividend: 12.7,
        stock_dividend: 0.0,
        share_ratio: 1.0,
    };
    let response = build_response("2330".to_owned(), 3, &raw, &[], &[dividend]);
    let closes: Vec<f64> = response.points.iter().map(|point| point.close).collect();
    // 前收 127、除息 12.7 → 因子 0.9：127 → 114.3；除息日 128 與最新一日 129 不變。
    assert_eq!(closes, [114.3, 128.0, 129.0]);
}

/// 日線少於 days 時全部回傳；沒有日線時資料日為 null、沒有訊號。
#[test]
fn build_response_handles_short_and_empty_history() {
    assert_eq!(
        build_response("2330".to_owned(), 20, &bars(2), &[], &[])
            .points
            .len(),
        2
    );
    let empty = build_response("2330".to_owned(), 20, &[], &[], &[]);
    assert!(empty.data_as_of.is_none());
    assert!(empty.points.is_empty() && empty.signals.is_empty());
}
