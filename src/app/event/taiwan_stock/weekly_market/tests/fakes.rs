//! 每週市場摘要測試用的假倉儲與固定資料。

use super::*;

/// 回傳固定加權指數收盤與市場廣度的假倉儲；`fail` 時指數查詢失敗，`breadth_fail` 時廣度查詢失敗。
#[derive(Default)]
pub(super) struct FakeIndex {
    pub(super) points: Vec<(NaiveDate, f64)>,
    pub(super) fail: bool,
    pub(super) breadth: Vec<BreadthDay>,
    pub(super) breadth_fail: bool,
}

#[async_trait]
impl BreadthQuery for FakeIndex {
    async fn fetch_breadth(
        &self,
        until: Option<NaiveDate>,
        limit: usize,
    ) -> Result<Vec<BreadthDay>> {
        assert_eq!((until, limit), (Some(day(16)), WEEK_DAYS + 1));
        if self.breadth_fail {
            return Err(anyhow!("breadth down"));
        }
        Ok(self.breadth.iter().copied().take(limit).collect())
    }
}

#[async_trait]
impl IndexCloseQuery for FakeIndex {
    async fn fetch_index_closes(
        &self,
        category: &str,
        until: Option<NaiveDate>,
        limit: usize,
    ) -> Result<Vec<(NaiveDate, f64)>> {
        assert_eq!((category, until), (regime::TAIEX, Some(day(17))));
        if self.fail {
            return Err(anyhow!("index down"));
        }
        Ok(self.points.iter().copied().take(limit).collect())
    }
}

/// 加權指數由新到舊：最新一天 2026-10-16 收盤 300，往前每天少 1，共 300 天（一路上漲，
/// 多頭排列、沒有穿越）。
pub(super) fn index_points() -> Vec<(NaiveDate, f64)> {
    (0..300u32)
        .map(|offset| {
            (
                day(16) - chrono::Days::new(u64::from(offset)),
                300.0 - f64::from(offset),
            )
        })
        .collect()
}

pub(super) fn index() -> FakeIndex {
    FakeIndex {
        points: index_points(),
        ..FakeIndex::default()
    }
}

pub(super) fn failing_index() -> FakeIndex {
    FakeIndex {
        fail: true,
        ..FakeIndex::default()
    }
}

/// 依查詢日回傳全體分數、交易日與持股分數的假倉儲。
#[derive(Default)]
pub(super) struct FakeQuote {
    pub(super) universes: HashMap<NaiveDate, (NaiveDate, Vec<ScoredStock>)>,
    pub(super) dates: Vec<NaiveDate>,
    pub(super) rs: Vec<RelativeStrengthDay>,
    pub(super) crossings: Vec<PeriodCrossing>,
    pub(super) fail: bool,
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

pub(super) fn crossing(symbol: &str, threshold: u8, up: bool, score: Option<u8>) -> PeriodCrossing {
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
pub(super) struct FakeChip {
    pub(super) dates: Vec<NaiveDate>,
    pub(super) flows: HashMap<String, Vec<StockFlow>>,
    pub(super) fail: bool,
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
pub(super) fn quote() -> FakeQuote {
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

pub(super) fn chip() -> FakeChip {
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

pub(super) fn holdings() -> BTreeSet<String> {
    ["1101", "2317", "2330", "2454"]
        .map(str::to_owned)
        .into_iter()
        .collect()
}

pub(super) fn name(symbol: &str) -> String {
    match symbol {
        "2330" => "台積電",
        "2317" => "鴻海",
        _ => "其他",
    }
    .to_owned()
}
