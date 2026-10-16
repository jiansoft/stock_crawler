//! 大盤趨勢變化通知的測試：穿越、排列改變、市場廣度跨過門檻、沒有變化、不是今天與查詢失敗。

use anyhow::anyhow;
use async_trait::async_trait;

use super::*;

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).expect("合法日期")
}

fn esc(line: &str) -> String {
    text::escape_markdown_v2(line)
}

/// 由新到舊的收盤，最新一天為 2026-10-16。
fn series(closes: &[f64]) -> Vec<(NaiveDate, f64)> {
    closes
        .iter()
        .enumerate()
        .map(|(offset, close)| (day(16) - chrono::Days::new(offset as u64), *close))
        .collect()
}

/// 最新一天跌到 100、前一天 400，再之前往前每天高 1：當天跌破 20／60 日均線；
/// 只有 102 天，240 日均線資料不足、排列兩天都是糾結。
fn breakdown() -> Vec<(NaiveDate, f64)> {
    let mut closes = vec![100.0, 400.0];
    closes.extend((2..102u32).map(|offset| 200.0 + f64::from(offset)));
    series(&closes)
}

/// 最新一天漲到 1000、前一天 250（跌破 20 日均線，排列糾結），再之前一路上漲：
/// 當天站上 60 日均線，排列轉為多頭。
fn breakout() -> Vec<(NaiveDate, f64)> {
    let mut closes = vec![1000.0, 250.0];
    closes.extend((2..300u32).map(|offset| 300.0 - f64::from(offset)));
    series(&closes)
}

/// 跌破 60 日均線：附均線值、距離與之前在上方的天數；20 日均線不在通知範圍。
#[test]
fn build_message_reports_breakdown() {
    let message = build_message(&breakdown(), &[]).expect("有跌破");
    for line in [
        "大盤趨勢變化︰2026-10-16",
        "加權指數收盤 100.00（-75.00%）",
        "• 跌破 60 日均線 231.15（收盤在下方 56.74%），之前連續 1 個交易日在上方",
        "＊只描述價格與均線的位置，不代表後續走勢",
    ] {
        assert!(message.contains(&esc(line)), "缺少 {line}：{message}");
    }
    assert!(!message.contains("20 日"), "{message}");
    assert!(!message.contains("均線排列"), "{message}");
}

/// 站上 60 日均線並轉為多頭排列。
#[test]
fn build_message_reports_breakout_and_alignment() {
    let message = build_message(&breakout(), &[]).expect("有站上");
    for line in [
        "加權指數收盤 1000.00（+300.00%）",
        "• 站上 60 日均線 281.35（收盤在上方 255.43%），之前連續 1 個交易日在下方",
        "• 均線排列：均線糾結 → 多頭排列",
    ] {
        assert!(message.contains(&esc(line)), "缺少 {line}：{message}");
    }
    assert!(!message.contains("240 日"), "{message}");
}

/// 一路上漲沒有穿越、排列不變，或只有一天：不發。
#[test]
fn build_message_skips_without_change() {
    let closes: Vec<f64> = (0..300).map(|offset| 300.0 - f64::from(offset)).collect();
    assert!(build_message(&series(&closes), &[]).is_none());
    assert!(build_message(&series(&[100.0]), &[]).is_none());
    assert!(build_message(&[], &[]).is_none());
}

/// 一路上漲、沒有穿越也沒有排列改變的收盤。
fn steady() -> Vec<(NaiveDate, f64)> {
    let closes: Vec<f64> = (0..300).map(|offset| 300.0 - f64::from(offset)).collect();
    series(&closes)
}

/// 站上 60 日均線的家數為 `above`（共 100 家）的一天。
fn breadth_day(d: u32, above: i32) -> BreadthDay {
    BreadthDay {
        date: day(d),
        above_ma20: 0,
        below_ma20: 0,
        above_ma60: above,
        below_ma60: 100 - above,
        above_ma240: 0,
        below_ma240: 0,
        new_high: 0,
        new_low: 0,
    }
}

/// 加權指數沒有變化，但市場廣度跨過 30%：跌破、回到都發；剛好 30% 算在線上。
#[test]
fn build_message_reports_breadth_cross() {
    let message = build_message(&steady(), &[breadth_day(16, 28), breadth_day(15, 31)])
        .expect("廣度跌破 30%");
    let line = "• 市場廣度：站上 60 日均線的股票 28.00%（前一日 31.00%），跌破 30%";
    assert!(message.contains(&esc(line)), "{message}");
    assert!(!message.contains("均線排列"), "{message}");

    let message = build_message(&steady(), &[breadth_day(16, 30), breadth_day(15, 29)])
        .expect("廣度回到 30%");
    assert!(
        message.contains(&esc("（前一日 29.00%），回到 30% 以上")),
        "{message}"
    );

    // 都在線的同一側：不發。
    assert!(build_message(&steady(), &[breadth_day(16, 45), breadth_day(15, 52)]).is_none());
    assert!(build_message(&steady(), &[breadth_day(16, 20)]).is_none());
}

/// 因加權指數穿越而發送時附上當天廣度，沒有跨過門檻就不註明方向。
#[test]
fn build_message_appends_breadth() {
    let message =
        build_message(&breakdown(), &[breadth_day(16, 45), breadth_day(15, 52)]).expect("有跌破");
    let line = "• 市場廣度：站上 60 日均線的股票 45.00%（前一日 52.00%）";
    assert!(message.contains(&format!("{}\n", esc(line))), "{message}");
}

/// 穿越的那條均線沒有值時不列（防禦用；正常穿越一定有均線）。
#[test]
fn cross_line_needs_moving_average() {
    let today = regime::regime(&series(&[100.0])).expect("有資料");
    assert!(cross_line(&today, None, 60, true).is_none());
    assert!(cross_line(&today, None, 10, true).is_none());
}

/// 回傳固定收盤與市場廣度的假倉儲；`fail` 時收盤查詢失敗、`breadth_fail` 時廣度查詢失敗。
struct FakeIndex {
    points: Vec<(NaiveDate, f64)>,
    fail: bool,
    breadth: Vec<BreadthDay>,
    breadth_fail: bool,
}

impl FakeIndex {
    fn new(points: Vec<(NaiveDate, f64)>) -> Self {
        Self {
            points,
            fail: false,
            breadth: Vec::new(),
            breadth_fail: false,
        }
    }
}

#[async_trait]
impl BreadthQuery for FakeIndex {
    async fn fetch_breadth(
        &self,
        until: Option<NaiveDate>,
        limit: usize,
    ) -> Result<Vec<BreadthDay>> {
        if self.breadth_fail {
            return Err(anyhow!("stats down"));
        }
        Ok(self
            .breadth
            .iter()
            .copied()
            .filter(|day| until.is_none_or(|until| day.date <= until))
            .take(limit)
            .collect())
    }
}

#[async_trait]
impl IndexCloseQuery for FakeIndex {
    async fn fetch_index_closes(
        &self,
        category: &str,
        until: Option<NaiveDate>,
        _limit: usize,
    ) -> Result<Vec<(NaiveDate, f64)>> {
        assert_eq!(category, regime::TAIEX);
        if self.fail {
            return Err(anyhow!("index down"));
        }
        Ok(self
            .points
            .iter()
            .copied()
            .filter(|(date, _)| until.is_none_or(|until| *date <= until))
            .collect())
    }
}

/// 今天有資料才組訊息；最新一筆不是今天（休市、尚未寫入）不發；查詢失敗回錯誤。
#[tokio::test]
async fn run_requires_today() {
    let fake = FakeIndex::new(breakdown());
    assert!(run(&fake, day(16)).await.expect("查詢成功").is_some());
    // 10-17 沒有資料：最新一筆是 10-16，不發。
    assert!(run(&fake, day(17)).await.expect("查詢成功").is_none());
    let empty = FakeIndex::new(Vec::new());
    assert!(run(&empty, day(16)).await.expect("查詢成功").is_none());
    let failing = FakeIndex {
        fail: true,
        ..FakeIndex::new(Vec::new())
    };
    assert!(run(&failing, day(16)).await.is_err());
}

/// 只有今天的廣度才看；廣度跨過門檻就發、讀取失敗只略過廣度。
#[tokio::test]
async fn run_reads_today_breadth() {
    let crossing = FakeIndex {
        breadth: vec![breadth_day(16, 28), breadth_day(15, 31)],
        ..FakeIndex::new(steady())
    };
    let message = run(&crossing, day(16)).await.expect("查詢成功");
    assert!(message.is_some_and(|message| message.contains("跌破 30%")));

    // 最新的統計是 10-15（今天還沒寫入）：不看廣度，加權指數沒變化就不發。
    let stale = FakeIndex {
        breadth: vec![breadth_day(15, 28), breadth_day(14, 31)],
        ..FakeIndex::new(steady())
    };
    assert!(run(&stale, day(16)).await.expect("查詢成功").is_none());

    let failing = FakeIndex {
        breadth_fail: true,
        ..FakeIndex::new(breakdown())
    };
    let message = run(&failing, day(16)).await.expect("廣度失敗不影響通知");
    assert!(message.is_some_and(|message| !message.contains("市場廣度")));
}
