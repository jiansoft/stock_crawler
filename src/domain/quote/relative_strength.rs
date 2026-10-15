//! 相對強度（RS）：近 60／120／240 日還原報酬加權後在上市櫃股票（不含 ETF）中的百分位分數。
//!
//! 加權報酬＝0.4 × 近 60 日 ＋ 0.3 × 近 120 日 ＋ 0.3 × 近 240 日（近 N 日報酬取
//! [`super::forward_return`] 基準日 T−N 的 N 日欄位），分數 1–99，99 為最強；分數、名次一律以
//! 當天上市櫃股票（不含 ETF）計算。本模組只放純計算與查詢介面，SQL 在 infra。
//!
//! 計分母體不含 ETF、ETN 與受益證券（產業 9001～9003）：股票只與股票比較，這些商品一律沒有分數。

use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap},
};

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

use super::technical::{
    RS50_INDICATOR, RS80_INDICATOR, SignalKind, StockTechnicalSignal, TechnicalSignal,
};

/// 計分用的最長天期（交易日數）。
pub const LONGEST: usize = 240;

/// 加權報酬中近 60、120、240 日報酬的權重。
pub const WEIGHTS: [f64; 3] = [0.4, 0.3, 0.3];

/// 穿越時要標記的分數門檻：80 為強勢、50 為中位（高門檻在前）。
pub const THRESHOLDS: [u8; 2] = [80, 50];

/// 每日技術訊號中，穿越 [`THRESHOLDS`] 各門檻的訊號來源（一一對應）。
pub const CROSSING_INDICATORS: [&str; 2] = [RS80_INDICATOR, RS50_INDICATOR];

/// 一檔股票某個交易日的相對強度。
#[derive(Debug, Clone, PartialEq)]
pub struct RelativeStrengthDay {
    /// 股票代號。
    pub stock_symbol: String,
    /// 交易日。
    pub date: NaiveDate,
    /// 加權報酬（%）。
    pub weighted: f64,
    /// 相對強度分數 1–99。
    pub rs_score: u8,
    /// 名次（比它強的家數＋1，同分同名次）。
    pub rank: i64,
    /// 當天計分的股票總數。
    pub universe: i64,
}

/// 比它弱的家數 `weaker` 在全體 `total` 中的百分位，換算成 1–99 分（只有一檔時為 99）。
pub fn percentile_score(weaker: usize, total: usize) -> u8 {
    if total <= 1 {
        return 99;
    }
    let rank = weaker as f64 / (total - 1) as f64;
    // rank 在 0–1 之間，換算後必在 1–99。
    (rank * 98.0).floor() as u8 + 1
}

/// 穿越門檻的方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossDirection {
    /// 由門檻下方升到門檻以上（含）。
    Up,
    /// 由門檻以上（含）跌到門檻下方。
    Down,
}

impl CrossDirection {
    /// API 使用的名稱：`up` 或 `down`。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

/// 前一個交易日分數 `previous` 到當天分數 `current` 穿越的 [`THRESHOLDS`]，高門檻在前。
pub fn crossings(previous: u8, current: u8) -> Vec<(u8, CrossDirection)> {
    THRESHOLDS
        .into_iter()
        .filter_map(|threshold| {
            match (previous >= threshold, current >= threshold) {
                (false, true) => Some(CrossDirection::Up),
                (true, false) => Some(CrossDirection::Down),
                _ => None,
            }
            .map(|direction| (threshold, direction))
        })
        .collect()
}

/// 依前一個交易日的分數 `previous`（代號 → 分數），產生 `date` 當天穿越 [`THRESHOLDS`] 的
/// 技術訊號：由下往上為 `cross_above`、由上往下為 `cross_below`，來源見 [`CROSSING_INDICATORS`]；
/// 說明附當天名次與計分總數（`scored` 是當天全體）。前一天沒有分數（剛滿 240 個交易日、
/// 前一天暫停交易）的股票不產生訊號。
pub fn crossing_signals(
    date: NaiveDate,
    previous: &HashMap<String, u8>,
    scored: &[ScoredStock],
) -> Vec<StockTechnicalSignal> {
    let universe = scored.len();
    let mut signals = Vec::new();
    for stock in scored {
        let Some(&before) = previous.get(&stock.row.stock_symbol) else {
            continue;
        };
        for (threshold, direction) in crossings(before, stock.rs_score) {
            let (kind, verb) = match direction {
                CrossDirection::Up => (SignalKind::CrossAbove, "突破"),
                CrossDirection::Down => (SignalKind::CrossBelow, "跌破"),
            };
            let indicator = THRESHOLDS
                .into_iter()
                .zip(CROSSING_INDICATORS)
                .find_map(|(known, indicator)| (known == threshold).then_some(indicator))
                .unwrap_or(RS50_INDICATOR);
            signals.push(StockTechnicalSignal {
                date,
                stock_symbol: stock.row.stock_symbol.clone(),
                signal: TechnicalSignal {
                    indicator,
                    kind,
                    description: format!(
                        "相對強度 {before} → {} 分，{verb} {threshold} 分（第 {} 名／{universe} 檔）",
                        stock.rs_score, stock.rank
                    ),
                },
            });
        }
    }
    signals
}

/// 計分母體中的一檔股票。
#[derive(Debug, Clone, PartialEq)]
pub struct UniverseRow {
    pub stock_symbol: String,
    pub name: String,
    pub market_id: i32,
    pub industry: String,
    pub return_20d: Option<f64>,
    pub return_60d: f64,
    pub return_120d: f64,
    pub return_240d: f64,
    pub closing_price: Option<f64>,
}

/// 已在全體中計分排名的一檔股票。
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredStock {
    /// 名次（比它強的家數＋1，同分同名次）。
    pub rank: usize,
    /// 相對強度分數 1–99。
    pub rs_score: u8,
    /// 加權報酬（%）。
    pub weighted: f64,
    pub row: UniverseRow,
}

impl UniverseRow {
    /// 加權報酬（%）。
    pub fn weighted(&self) -> f64 {
        WEIGHTS[0] * self.return_60d + WEIGHTS[1] * self.return_120d + WEIGHTS[2] * self.return_240d
    }
}

/// 資料日與各天期的基準日。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BaseDates {
    /// 資料日。
    pub end: NaiveDate,
    /// 往前第 20 個交易日。
    pub d20: NaiveDate,
    /// 往前第 60 個交易日。
    pub d60: NaiveDate,
    /// 往前第 120 個交易日。
    pub d120: NaiveDate,
    /// 往前第 240 個交易日。
    pub d240: NaiveDate,
}

/// 以 `trading_dates`（由新到舊）的第一天為資料日，找出各天期的基準日；交易日不足回 `None`。
pub fn base_dates(trading_dates: &[NaiveDate]) -> Option<BaseDates> {
    let end = *trading_dates.first()?;
    let base = |horizon| super::forward_return::base_date(end, horizon, trading_dates);
    Some(BaseDates {
        end,
        d20: base(20)?,
        d60: base(60)?,
        d120: base(120)?,
        d240: base(LONGEST)?,
    })
}

/// 對全體計分與排名，由強到弱；同分同名次，並依代號排列讓輸出穩定。
pub fn score_universe(rows: Vec<UniverseRow>) -> Vec<ScoredStock> {
    let universe = rows.len();
    let mut weighted: Vec<(f64, UniverseRow)> =
        rows.into_iter().map(|row| (row.weighted(), row)).collect();
    weighted.sort_by(|(a, row_a), (b, row_b)| {
        b.total_cmp(a)
            .then_with(|| row_a.stock_symbol.cmp(&row_b.stock_symbol))
    });

    let mut scored = Vec::with_capacity(universe);
    let mut group_start = 0;
    while group_start < weighted.len() {
        let value = weighted[group_start].0;
        let group_end = weighted[group_start..]
            .iter()
            .position(|(other, _)| other.total_cmp(&value) != Ordering::Equal)
            .map_or(weighted.len(), |len| group_start + len);
        // 名次＝比它強的家數＋1；分數＝比它弱的家數占其他家數的百分位。
        let rs_score = percentile_score(weighted.len() - group_end, universe);
        scored.extend(
            weighted[group_start..group_end]
                .iter()
                .map(|(_, row)| ScoredStock {
                    rank: group_start + 1,
                    rs_score,
                    weighted: value,
                    row: row.clone(),
                }),
        );
        group_start = group_end;
    }
    scored
}

/// 依市場篩選後把股票分到各產業（未分類略過），保持由強到弱的順序。
pub fn group_by_industry(
    scored: Vec<ScoredStock>,
    market_id: i32,
) -> BTreeMap<String, Vec<ScoredStock>> {
    let mut groups: BTreeMap<String, Vec<ScoredStock>> = BTreeMap::new();
    for stock in scored {
        if stock.row.industry.is_empty() || (market_id != 0 && stock.row.market_id != market_id) {
            continue;
        }
        groups
            .entry(stock.row.industry.clone())
            .or_default()
            .push(stock);
    }
    groups
}

/// 分數中位數；偶數檔取中間兩檔的平均。
pub fn median_score(stocks: &[ScoredStock]) -> Option<f64> {
    let mut scores: Vec<u8> = stocks.iter().map(|stock| stock.rs_score).collect();
    scores.sort_unstable();
    let middle = scores.len() / 2;
    match scores.len() {
        0 => None,
        len if len % 2 == 1 => Some(f64::from(scores[middle])),
        _ => Some((f64::from(scores[middle - 1]) + f64::from(scores[middle])) / 2.0),
    }
}

/// 相對強度歷史的查詢介面。
#[async_trait]
pub trait RelativeStrengthQuery: Send + Sync {
    /// `until`（含）之前最近 `days` 個已寫入每日分數的交易日，取 `stock_symbols` 的分數，
    /// 依代號、再由新到舊排列。上市未滿 240 個交易日、暫停交易、ETF 或非上市櫃的股票沒有分數。
    async fn fetch_relative_strength(
        &self,
        until: NaiveDate,
        days: u16,
        stock_symbols: &[String],
    ) -> Result<Vec<RelativeStrengthDay>>;
}

/// 每日分數的寫入介面。
#[async_trait]
pub trait RelativeStrengthRepository: Send + Sync {
    /// 在同一個交易內以 `scored`（當天全體分數）整天替換 `date` 的每日分數，並以 `signals`
    /// 替換當天的相對強度穿越訊號（[`CROSSING_INDICATORS`]，其他技術訊號不動），回傳分數寫入列數。
    async fn replace_relative_strength(
        &self,
        date: NaiveDate,
        scored: &[ScoredStock],
        signals: &[StockTechnicalSignal],
    ) -> Result<u64>;
}

/// 期間內某檔股票對某個門檻的最後一次穿越（取自 [`CROSSING_INDICATORS`] 訊號）。
#[derive(Debug, Clone, PartialEq)]
pub struct PeriodCrossing {
    /// 股票代號。
    pub stock_symbol: String,
    /// 穿越的門檻（[`THRESHOLDS`] 之一）。
    pub threshold: u8,
    /// 期間內最後一次穿越的方向。
    pub direction: CrossDirection,
    /// 期間最後一天的分數；當天沒有分數時為 `None`。
    pub latest_score: Option<u8>,
}

/// 由訊號來源與種類還原門檻與方向；不是相對強度穿越訊號時回 `None`。
pub fn parse_crossing(indicator: &str, kind: &str) -> Option<(u8, CrossDirection)> {
    let threshold = THRESHOLDS
        .into_iter()
        .zip(CROSSING_INDICATORS)
        .find_map(|(threshold, known)| (known == indicator).then_some(threshold))?;
    let direction = match SignalKind::parse(kind)? {
        SignalKind::CrossAbove => CrossDirection::Up,
        SignalKind::CrossBelow => CrossDirection::Down,
        _ => return None,
    };
    Some((threshold, direction))
}

/// 相對強度穿越訊號的期間查詢介面。
#[async_trait]
pub trait RelativeStrengthCrossingQuery: Send + Sync {
    /// `from`～`to`（皆含）每檔股票對每個門檻的最後一次穿越，附 `to` 當天的分數。
    async fn fetch_period_crossings(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<PeriodCrossing>>;
}

/// 全體計分的查詢介面。
#[async_trait]
pub trait ScoredUniverseQuery: Send + Sync {
    /// `until`（含）之前最近一個已算好報酬的交易日的上市櫃股票（不含 ETF）分數，由強到弱。
    ///
    /// 最新一個交易日的報酬可能還沒回填（例如收盤匯總尚未跑完），最多往前退一天；兩天都沒有
    /// 資料回 `Ok(None)`。
    async fn fetch_scored_universe(
        &self,
        until: NaiveDate,
    ) -> Result<Option<(NaiveDate, Vec<ScoredStock>)>>;
}

#[cfg(test)]
mod tests {
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
}
