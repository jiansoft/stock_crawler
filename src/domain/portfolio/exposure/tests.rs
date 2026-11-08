//! 持股穿透曝險：把持有 ETF 的市值依前十大持股權重拆到成分股，與直接持股合併的測試。

use super::*;

fn position(symbol: &str, name: &str, value: Option<f64>, is_etf: bool) -> ExposurePosition {
    ExposurePosition {
        stock_symbol: symbol.to_owned(),
        name: name.to_owned(),
        market_value: value,
        is_etf,
    }
}

fn constituent(etf: &str, symbol: &str, name: &str, weight: f64) -> EtfConstituent {
    EtfConstituent {
        etf_symbol: etf.to_owned(),
        as_of: NaiveDate::from_ymd_opt(2026, 9, 30).expect("日期合法"),
        holding_symbol: symbol.to_owned(),
        holding_name: name.to_owned(),
        weight,
    }
}

fn sample() -> (Vec<ExposurePosition>, Vec<EtfConstituent>) {
    (
        vec![
            position("2330", "台積電", Some(100_000.0), false),
            position("0050", "元大台灣50", Some(200_000.0), true),
            position("0056", "元大高股息", Some(50_000.0), true),
            position("00679B", "元大美債20年", Some(10_000.0), true),
            position("2887Z1", "台新戊特", None, false),
        ],
        vec![
            constituent("0050", "2330", "台積電", 56.0),
            constituent("0050", "2454", "聯發科", 6.0),
            constituent("0056", "2454", "聯發科", 8.0),
            constituent("0056", "", "現金", 2.0),
        ],
    )
}

#[test]
fn etf_value_is_split_by_top_holdings_and_merged_with_direct_positions() {
    let (positions, constituents) = sample();
    let report = look_through(&positions, &constituents);
    assert_eq!(report.total_value, 360_000.0);
    assert_eq!(report.etf_value, 260_000.0);
    assert_eq!(report.unpriced, ["2887Z1"]);

    let tsmc = &report.exposures[0];
    assert_eq!(
        (tsmc.symbol.as_str(), tsmc.name.as_str()),
        ("2330", "台積電")
    );
    assert_eq!(tsmc.direct_value, 100_000.0);
    assert_eq!(tsmc.etf_value(), 112_000.0);
    assert_eq!(tsmc.total_value(), 212_000.0);

    let mediatek = &report.exposures[1];
    assert_eq!(mediatek.symbol, "2454");
    assert_eq!(mediatek.direct_value, 0.0);
    let sources: Vec<(&str, f64)> = mediatek
        .sources
        .iter()
        .map(|s| (s.etf_symbol.as_str(), s.value))
        .collect();
    assert_eq!(sources, [("0050", 12_000.0), ("0056", 4_000.0)]);

    // 沒有代號的持股以名稱比對。
    assert_eq!(report.exposures[2].name, "現金");
    assert_eq!(report.exposures[2].symbol, "");
    assert_eq!(report.exposures.len(), 3);
}

#[test]
fn direct_only_positions_have_positive_zero_etf_value() {
    let report = look_through(&[position("2887", "台新新光金", Some(1.0), false)], &[]);
    let value = report.exposures[0].etf_value();
    assert!(value == 0.0 && value.is_sign_positive(), "{value}");
}

#[test]
fn uncovered_value_counts_holdings_outside_the_top_ten() {
    let (positions, constituents) = sample();
    let report = look_through(&positions, &constituents);
    // 0050：38% × 200,000；0056：90% × 50,000；00679B 沒有前十大：全部。
    assert!((report.uncovered_value - (76_000.0 + 45_000.0 + 10_000.0)).abs() < 1e-6);
    let symbols: Vec<&str> = report
        .etfs
        .iter()
        .map(|e| e.stock_symbol.as_str())
        .collect();
    assert_eq!(symbols, ["0050", "0056", "00679B"]);
    assert_eq!(report.etfs[0].covered_weight, 62.0);
    assert!(report.etfs[2].holdings_as_of.is_none());
    assert!(report.etfs[0].holdings_as_of.is_some());
}

#[test]
fn weights_over_one_hundred_never_make_uncovered_negative() {
    let report = look_through(
        &[position("00631L", "元大台灣50正2", Some(1_000.0), true)],
        &[constituent("00631L", "", "期貨保證金", 100.5)],
    );
    assert_eq!(report.uncovered_value, 0.0);
}

struct FakeRepo(Vec<ExposurePosition>, Vec<EtfConstituent>);

#[async_trait]
impl EtfExposureQuery for FakeRepo {
    async fn fetch_exposure_positions(&self, member_id: i64) -> Result<Vec<ExposurePosition>> {
        assert_eq!(member_id, 7);
        Ok(self.0.clone())
    }

    async fn fetch_etf_constituents(&self, etf_symbols: &[String]) -> Result<Vec<EtfConstituent>> {
        assert_eq!(etf_symbols, ["0050", "0056", "00679B"], "沒有收盤價的不查");
        Ok(self.1.clone())
    }
}

#[tokio::test]
async fn build_report_reads_constituents_of_priced_etfs_only() {
    let (positions, constituents) = sample();
    let report = build_report(&FakeRepo(positions, constituents), 7)
        .await
        .expect("計算成功");
    assert_eq!(report.exposures[0].total_value(), 212_000.0);

    // 沒有持有 ETF 時不查前十大持股（FakeRepo 的斷言會失敗）。
    let only_stocks = vec![position("2330", "台積電", Some(1.0), false)];
    struct NoEtf(Vec<ExposurePosition>);
    #[async_trait]
    impl EtfExposureQuery for NoEtf {
        async fn fetch_exposure_positions(&self, _: i64) -> Result<Vec<ExposurePosition>> {
            Ok(self.0.clone())
        }
        async fn fetch_etf_constituents(&self, _: &[String]) -> Result<Vec<EtfConstituent>> {
            panic!("沒有 ETF 不應查詢前十大持股");
        }
    }
    let report = build_report(&NoEtf(only_stocks), 0)
        .await
        .expect("計算成功");
    assert_eq!(report.etf_value, 0.0);
    assert_eq!(report.exposures.len(), 1);
}
