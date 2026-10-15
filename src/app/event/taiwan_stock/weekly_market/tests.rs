//! 每週市場摘要的測試：產業 RS 中位數與一週變化、產業法人淨買賣、持股 RS 變化、降級與訊息格式。

use std::collections::HashMap;

use anyhow::anyhow;
use async_trait::async_trait;

use super::*;
use crate::domain::quote::relative_strength::{ScoredStock, UniverseRow};

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("合法日期")
}

fn esc(line: &str) -> String {
    text::escape_markdown_v2(line)
}

fn scored(symbol: &str, industry: &str, rs_score: u8) -> ScoredStock {
    ScoredStock {
        rank: 1,
        rs_score,
        weighted: f64::from(rs_score),
        row: UniverseRow {
            stock_symbol: symbol.to_owned(),
            name: symbol.to_owned(),
            market_id: 2,
            industry: industry.to_owned(),
            return_20d: None,
            return_60d: 0.0,
            return_120d: 0.0,
            return_240d: 0.0,
            closing_price: None,
        },
    }
}

/// 同一產業的多檔股票，代號以產業名加序號。
fn industry(name: &str, scores: &[u8]) -> Vec<ScoredStock> {
    scores
        .iter()
        .enumerate()
        .map(|(index, score)| scored(&format!("{name}{index}"), name, *score))
        .collect()
}

/// universe 101 時分數＝weaker × 98 ÷ 100 取整＋1。
fn rs_day(symbol: &str, d: u32, weaker: i64) -> RelativeStrengthDay {
    RelativeStrengthDay {
        stock_symbol: symbol.to_owned(),
        date: day(d),
        weighted: 0.0,
        rs_score: crate::domain::quote::relative_strength::percentile_score(
            usize::try_from(weaker).expect("測試資料非負"),
            101,
        ),
        rank: 101 - weaker,
        universe: 101,
    }
}

fn flow(symbol: &str, industry: &str, amount: f64) -> StockFlow {
    StockFlow {
        stock_symbol: symbol.to_owned(),
        name: symbol.to_owned(),
        market_id: 2,
        industry: industry.to_owned(),
        net_shares: 0,
        net_amount: amount,
        volume: None,
        trade_value: None,
        buy_days: 0,
        sell_days: 0,
        streak: 0,
        closing_price: None,
    }
}

/// 依查詢日回傳全體分數、交易日與持股分數的假倉儲。
#[derive(Default)]
struct FakeQuote {
    universes: HashMap<NaiveDate, (NaiveDate, Vec<ScoredStock>)>,
    dates: Vec<NaiveDate>,
    rs: Vec<RelativeStrengthDay>,
    crossings: Vec<PeriodCrossing>,
    fail: bool,
}

#[async_trait]
impl RelativeStrengthCrossingQuery for FakeQuote {
    async fn fetch_period_crossings(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<PeriodCrossing>> {
        assert_eq!((from, to), (day(12), day(16)), "最近 5 個交易日");
        if self.fail {
            return Err(anyhow!("連線中斷"));
        }
        Ok(self.crossings.clone())
    }
}

fn crossing(symbol: &str, threshold: u8, up: bool, score: Option<u8>) -> PeriodCrossing {
    PeriodCrossing {
        stock_symbol: symbol.to_owned(),
        threshold,
        direction: if up {
            CrossDirection::Up
        } else {
            CrossDirection::Down
        },
        latest_score: score,
    }
}

#[async_trait]
impl ScoredUniverseQuery for FakeQuote {
    async fn fetch_scored_universe(
        &self,
        until: NaiveDate,
    ) -> Result<Option<(NaiveDate, Vec<ScoredStock>)>> {
        if self.fail {
            return Err(anyhow!("連線中斷"));
        }
        Ok(self.universes.get(&until).cloned())
    }
}

#[async_trait]
impl RelativeStrengthQuery for FakeQuote {
    async fn fetch_relative_strength(
        &self,
        _until: NaiveDate,
        days: u16,
        _stock_symbols: &[String],
    ) -> Result<Vec<RelativeStrengthDay>> {
        assert_eq!(usize::from(days), WEEK_DAYS + 1);
        if self.fail {
            return Err(anyhow!("連線中斷"));
        }
        Ok(self.rs.clone())
    }
}

#[async_trait]
impl ForwardReturnRepository for FakeQuote {
    async fn fetch_trading_dates(&self, _until: NaiveDate, count: usize) -> Result<Vec<NaiveDate>> {
        Ok(self.dates.iter().copied().take(count).collect())
    }

    async fn upsert_forward_returns(
        &self,
        _base: NaiveDate,
        _end: NaiveDate,
        _horizon: usize,
    ) -> Result<u64> {
        Ok(0)
    }
}

/// 依法人別回傳淨買賣的假籌碼查詢。
#[derive(Default)]
struct FakeChip {
    dates: Vec<NaiveDate>,
    flows: HashMap<String, Vec<StockFlow>>,
    fail: bool,
}

#[async_trait]
impl ChipFlowQuery for FakeChip {
    async fn fetch_chip_dates(&self, _until: NaiveDate) -> Result<Vec<NaiveDate>> {
        if self.fail {
            return Err(anyhow!("籌碼逾時"));
        }
        Ok(self.dates.clone())
    }

    async fn fetch_flows(&self, _period: FlowPeriod, investor: &str) -> Result<Vec<StockFlow>> {
        Ok(self.flows.get(investor).cloned().unwrap_or_default())
    }

    async fn fetch_streaks(&self, _data_date: NaiveDate) -> Result<Vec<flow::StockStreak>> {
        Ok(Vec::new())
    }
}

/// 週五 10-16 為資料日；往前第 5 個交易日是 10-09。
fn quote() -> FakeQuote {
    let mut current = industry("半導體業", &[90, 80, 70, 60, 50]);
    current.extend(industry("航運業", &[30, 40, 50, 60, 70]));
    current.extend(industry("光電業", &[20, 20, 30, 40, 90]));
    current.extend(industry("觀光業", &[99, 99]));
    current.push(scored("未分類", "", 10));
    let mut prior = industry("半導體業", &[60, 65, 70, 50, 80]);
    prior.extend(industry("航運業", &[60, 60, 60, 60, 60]));
    FakeQuote {
        universes: HashMap::from([(day(17), (day(16), current)), (day(9), (day(9), prior))]),
        dates: vec![day(16), day(15), day(14), day(13), day(12), day(9)],
        rs: vec![
            rs_day("1101", 16, 40),
            rs_day("1101", 9, 40),
            rs_day("2317", 16, 60),
            rs_day("2317", 9, 50),
            rs_day("2330", 16, 80),
            rs_day("2330", 9, 85),
            rs_day("2454", 16, 90),
        ],
        crossings: vec![
            crossing("2330", 80, true, Some(85)),
            crossing("2317", 80, true, Some(92)),
            crossing("2454", 80, true, None),
            crossing("6669", 80, true, Some(81)),
            crossing("1101", 50, false, Some(30)),
            crossing("2603", 50, false, Some(45)),
        ],
        fail: false,
    }
}

fn chip() -> FakeChip {
    FakeChip {
        dates: vec![day(16), day(15), day(14), day(13), day(12), day(9)],
        flows: HashMap::from([(
            "foreign".to_owned(),
            vec![
                flow("A", "半導體業", 3e9),
                flow("B", "半導體業", -1e9),
                flow("C", "航運業", -5e8),
                flow("D", "", 9e9),
                flow("E", "金融保險業", 1.2e9),
            ],
        )]),
        fail: false,
    }
}

fn holdings() -> BTreeSet<String> {
    ["1101", "2317", "2330", "2454"]
        .map(str::to_owned)
        .into_iter()
        .collect()
}

fn name(symbol: &str) -> String {
    match symbol {
        "2330" => "台積電",
        "2317" => "鴻海",
        _ => "其他",
    }
    .to_owned()
}

/// 產業中位數排名、一週變化（少於 5 檔與未分類不列）、轉強轉弱。
#[tokio::test]
async fn industry_section_ranks_medians_with_weekly_change() {
    let section = industry_section(&quote(), day(17))
        .await
        .expect("成功")
        .expect("有分數");
    assert_eq!(section.data_date, day(16));
    let medians: Vec<(&str, f64, Option<f64>)> = section
        .industries
        .iter()
        .map(|item| (item.industry.as_str(), item.median, item.change))
        .collect();
    assert_eq!(
        medians,
        [
            ("半導體業", 70.0, Some(5.0)),
            ("航運業", 50.0, Some(-10.0)),
            ("光電業", 30.0, None),
        ]
    );
    assert!(
        industry_section(&FakeQuote::default(), day(17))
            .await
            .expect("成功")
            .is_none()
    );
}

/// 完整訊息：三段內容、億元換算、持股上升下降、沒有變化或只有一筆的持股不列。
#[tokio::test]
async fn run_builds_full_message() {
    let message = run(&quote(), &chip(), day(17), &holdings())
        .await
        .expect("有內容");
    // name 由 holdings::stock_name 提供；這裡只驗證與名稱無關的段落。
    for line in [
        "每週市場摘要︰2026-10-16",
        "產業相對強度（成分股 RS 中位數，括號為與 5 個交易日前相比）",
        "• 最強：半導體業 70（+5.0）、航運業 50（-10.0）、光電業 30",
        "• 最弱：光電業 30、航運業 50（-10.0）、半導體業 70（+5.0）",
        "• 轉強：半導體業 +5.0",
        "• 轉弱：航運業 -10.0",
        "相對強度穿越（2026-10-12～2026-10-16，每檔只取最後一次；後面數字為最新分數）",
        "外資近 5 日產業淨買賣（2026-10-12～2026-10-16）",
        "• 流入：半導體業 +20.0 億、金融保險業 +12.0 億",
        "• 流出：航運業 -5.0 億",
        "持股相對強度（與 5 個交易日前相比）",
    ] {
        assert!(message.contains(&esc(line)), "缺少 {line}：{message}");
    }
    assert!(!message.contains("投信"), "投信沒有資料不列：{message}");
    assert!(!message.contains("觀光業"), "少於 5 檔的產業不列");
}

/// 穿越：站上依最新分數由高到低、跌破由低到高，各列最多 3 檔；沒有最新分數的排最後；
/// 沒有股票的門檻與方向不列。
#[test]
fn crossing_lines_rank_by_latest_score() {
    let crossings = quote().crossings;
    assert_eq!(
        crossing_line(&crossings, 80, CrossDirection::Up, &name).as_deref(),
        Some("• 站上 80 分 4 檔：2317 鴻海 92、2330 台積電 85、6669 其他 81")
    );
    assert_eq!(
        crossing_line(&crossings, 50, CrossDirection::Down, &name).as_deref(),
        Some("• 跌破 50 分 2 檔：1101 其他 30、2603 其他 45")
    );
    assert!(crossing_line(&crossings, 80, CrossDirection::Down, &name).is_none());
    let unscored = [crossing("2454", 80, true, None)];
    assert_eq!(
        crossing_line(&unscored, 80, CrossDirection::Up, &name).as_deref(),
        Some("• 站上 80 分 1 檔：2454 其他")
    );

    let section = CrossingSection {
        from: day(12),
        to: day(16),
        crossings,
    };
    let message = build_message(None, Some(&section), &[], &[], name).expect("有穿越");
    assert!(
        message.starts_with(&esc("每週市場摘要︰2026-10-16\n")),
        "{message}"
    );
    assert!(
        message.contains(&esc(
            "• 站上 80 分 4 檔：2317 鴻海 92、2330 台積電 85、6669 其他 81\n• 跌破 50 分 2 檔"
        )),
        "{message}"
    );
    let empty = CrossingSection {
        crossings: Vec::new(),
        ..section
    };
    assert!(build_message(None, Some(&empty), &[], &[], name).is_none());
}

/// 持股 RS 變化：依變化排序，上升與下降各列最多 3 檔，沒有變化的不列。
#[test]
fn build_message_lists_holding_moves() {
    let moves = holding_moves(&quote().rs);
    assert_eq!(
        moves,
        [
            HoldingMove {
                stock_symbol: "1101".to_owned(),
                from: 40,
                to: 40
            },
            HoldingMove {
                stock_symbol: "2317".to_owned(),
                from: 50,
                to: 59
            },
            HoldingMove {
                stock_symbol: "2330".to_owned(),
                from: 84,
                to: 79
            },
        ]
    );
    let message = build_message(None, None, &[], &moves, name).expect("有持股變化");
    assert!(message.starts_with(&esc("每週市場摘要\n")), "{message}");
    assert!(
        message.contains(&esc("• 上升：2317 鴻海 50 → 59（+9）")),
        "{message}"
    );
    assert!(
        message.contains(&esc("• 下降：2330 台積電 84 → 79（-5）")),
        "{message}"
    );
    assert!(!message.contains("1101"), "沒有變化的不列");
    assert!(build_message(None, None, &[], &[], name).is_none());
}

/// 各段查詢失敗只略過該段；全部失敗或沒有資料時不發送。
#[tokio::test]
async fn run_degrades_per_section() {
    let failing_chip = FakeChip {
        fail: true,
        ..chip()
    };
    let message = run(&quote(), &failing_chip, day(17), &holdings())
        .await
        .expect("產業與持股仍有內容");
    assert!(!message.contains("外資"), "{message}");
    assert!(message.contains(&esc("• 轉強：半導體業 +5.0")));

    let failing_quote = FakeQuote {
        fail: true,
        ..quote()
    };
    let message = run(&failing_quote, &chip(), day(17), &holdings())
        .await
        .expect("法人仍有內容");
    assert!(
        message.contains(&esc("每週市場摘要︰2026-10-16")),
        "{message}"
    );
    assert!(!message.contains("相對強度"), "{message}");

    assert!(
        run(&failing_quote, &failing_chip, day(17), &holdings())
            .await
            .is_none()
    );
}

/// 數字格式：分數變化與億元都帶正負號、一位小數。
#[test]
fn number_formats() {
    assert_eq!(signed(5.0), "+5.0");
    assert_eq!(signed(-2.24), "-2.2");
    assert_eq!(hundred_million(1.234e9), "+12.3 億");
    assert_eq!(hundred_million(-5e7), "-0.5 億");
}
