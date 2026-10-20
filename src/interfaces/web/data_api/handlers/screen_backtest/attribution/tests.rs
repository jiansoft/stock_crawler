//! 回測報酬歸因（Brinson-Fachler）的測試：超額報酬拆解、現金部位與跨期彙總。

use super::*;

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).expect("有效日期")
}

fn near(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-9
}

/// 策略：產業 1 的 A（報酬 10）、產業 2 的 B（報酬 −2），等權。
fn strategy() -> PeriodRow {
    PeriodRow {
        date: day(1, 2),
        holdings: 2,
        symbols: vec!["A".to_owned(), "B".to_owned()],
        returns: vec![Some(10.0), Some(-2.0)],
        industries: vec![1, 2],
        avg_return: Some(4.0),
        etf_return: Some(3.0),
        ..PeriodRow::default()
    }
}

/// 基準：產業 1 占 75%（報酬 4）、產業 3 占 25%（報酬 8）；整體 5。
fn benchmark() -> BenchmarkRow {
    BenchmarkRow {
        date: day(1, 2),
        industries: vec![1, 3, 1],
        returns: vec![4.0, 8.0, 4.0],
        trade_values: vec![50.0, 25.0, 25.0],
    }
}

/// 三項效果合計等於超額報酬；策略沒持有的產業 rp 取 rb、基準沒有的產業 rb 取 Rb。
#[test]
fn attribute_splits_excess_return() {
    let result = attribute(&strategy(), &benchmark()).expect("有基準");
    assert!(near(result.strategy_return, 4.0));
    assert!(near(result.benchmark_return, 5.0));
    assert_eq!(result.etf_return, Some(3.0));
    let sum: f64 = result.effects.values().map(Effect::total).sum();
    assert!(near(sum, -1.0), "{result:?}");

    let one = result.effects[&Bucket::Industry(1)];
    assert!(near(one.weight, 0.5) && near(one.benchmark_weight, 0.75));
    assert!(near(one.allocation, (0.5 - 0.75) * (4.0 - 5.0)));
    assert!(near(one.selection, 0.75 * (10.0 - 4.0)));
    assert!(near(one.interaction, (0.5 - 0.75) * (10.0 - 4.0)));
    // 基準沒有產業 2：rb 取 Rb，配置為 0、差額都在交互。
    let two = result.effects[&Bucket::Industry(2)];
    assert!(near(two.allocation, 0.0) && near(two.selection, 0.0));
    assert!(near(two.interaction, 0.5 * (-2.0 - 5.0)));
    // 策略沒持有產業 3：選股、交互為 0，少配的配置效果。
    let three = result.effects[&Bucket::Industry(3)];
    assert!(near(three.allocation, -0.25 * (8.0 - 5.0)));
    assert!(near(three.selection, 0.0) && near(three.interaction, 0.0));
    assert!(!result.effects.contains_key(&Bucket::Cash));

    assert!(attribute(&strategy(), &BenchmarkRow::default()).is_none());
}

/// 現金（單檔上限、空手、沒有報酬的持股）是報酬 0 的類別，效果全在配置；仍滿足合計等於超額報酬。
#[test]
fn attribute_treats_cash_as_zero_return_bucket() {
    let capped = PeriodRow {
        cash: 0.2,
        ..strategy()
    };
    let result = attribute(&capped, &benchmark()).expect("有基準");
    assert!(near(result.strategy_return, 3.2));
    let cash = result.effects[&Bucket::Cash];
    assert!(near(cash.weight, 0.2));
    assert!(near(cash.allocation, 0.2 * -5.0));
    let sum: f64 = result.effects.values().map(Effect::total).sum();
    assert!(near(sum, 3.2 - 5.0));

    // 一檔沒有報酬：另一檔放大到全部持股比例。
    let unpriced = PeriodRow {
        returns: vec![Some(10.0), None],
        ..strategy()
    };
    let result = attribute(&unpriced, &benchmark()).expect("有基準");
    assert!(near(result.strategy_return, 10.0));
    assert!(near(result.effects[&Bucket::Industry(1)].weight, 1.0));

    // 空手：全部是現金。
    let idle = PeriodRow {
        holdings: 0,
        symbols: Vec::new(),
        returns: Vec::new(),
        industries: Vec::new(),
        avg_return: None,
        ..strategy()
    };
    let result = attribute(&idle, &benchmark()).expect("有基準");
    assert!(near(result.strategy_return, 0.0));
    assert!(near(result.effects[&Bucket::Cash].allocation, -5.0));
}

/// 合計與產業輸出：各期相加、權重為各期平均；整段依合計排序、年度依絕對值取前幾個；名稱查無為
/// 「未分類」、現金為「現金」；0050 全缺為 null。
#[test]
fn totals_and_industry_effects_aggregate_periods() {
    let first = attribute(&strategy(), &benchmark()).expect("有基準");
    let second = PeriodAttribution {
        date: day(2, 1),
        etf_return: None,
        ..first.clone()
    };
    let (summary, buckets) = totals(&[&first, &second]);
    assert_eq!(summary.periods, 2);
    assert_eq!(summary.strategy_return, 8.0);
    assert_eq!(summary.benchmark_return, 10.0);
    assert_eq!(summary.etf_return, Some(3.0));
    assert_eq!(summary.excess_return, -2.0);
    assert!(near(
        summary.allocation + summary.selection + summary.interaction,
        -2.0
    ));
    let (empty, _) = totals(&[]);
    assert_eq!((empty.periods, empty.etf_return), (0, None));

    let names: HashMap<i32, String> = [(1, "半導體業".to_owned())].into_iter().collect();
    let all = industry_effects(&buckets, 2, &names, None);
    assert_eq!(all[0].industry, "半導體業");
    assert_eq!(all[0].avg_weight, 50.0);
    assert_eq!(all[0].benchmark_avg_weight, 75.0);
    assert!(all.windows(2).all(|pair| pair[0].total >= pair[1].total));
    assert!(all.iter().any(|item| item.industry == "未分類"));
    // 依絕對值：產業 2（合計 −7）大於產業 1（6.5）。
    let top = industry_effects(&buckets, 2, &names, Some(1));
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].industry, "未分類");
    assert_eq!(top[0].total, -7.0);

    let mut cash = BTreeMap::new();
    cash.insert(Bucket::Cash, Effect::default());
    assert_eq!(industry_effects(&cash, 0, &names, None)[0].industry, "現金");

    let years = attribution_years(&[first, second], &names);
    assert_eq!(years.len(), 1);
    assert_eq!(years[0].year, 2024);
    assert_eq!(years[0].totals.periods, 2);
    assert!(years[0].industries.len() <= YEAR_INDUSTRIES);
}
