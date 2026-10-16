//! 大盤趨勢變化通知的測試：穿越、排列改變、沒有變化、不是今天與查詢失敗。

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
    let message = build_message(&breakdown()).expect("有跌破");
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
    let message = build_message(&breakout()).expect("有站上");
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
    assert!(build_message(&series(&closes)).is_none());
    assert!(build_message(&series(&[100.0])).is_none());
    assert!(build_message(&[]).is_none());
}

/// 穿越的那條均線沒有值時不列（防禦用；正常穿越一定有均線）。
#[test]
fn cross_line_needs_moving_average() {
    let today = regime::regime(&series(&[100.0])).expect("有資料");
    assert!(cross_line(&today, None, 60, true).is_none());
    assert!(cross_line(&today, None, 10, true).is_none());
}

/// 回傳固定收盤的假倉儲；`fail` 時查詢失敗。
struct FakeIndex {
    points: Vec<(NaiveDate, f64)>,
    fail: bool,
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
    let fake = FakeIndex {
        points: breakdown(),
        fail: false,
    };
    assert!(run(&fake, day(16)).await.expect("查詢成功").is_some());
    // 10-17 沒有資料：最新一筆是 10-16，不發。
    assert!(run(&fake, day(17)).await.expect("查詢成功").is_none());
    let empty = FakeIndex {
        points: Vec::new(),
        fail: false,
    };
    assert!(run(&empty, day(16)).await.expect("查詢成功").is_none());
    let failing = FakeIndex {
        points: Vec::new(),
        fail: true,
    };
    assert!(run(&failing, day(16)).await.is_err());
}
