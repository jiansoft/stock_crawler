//! 上櫃股票減資恢復買賣參考價格的測試；連外的測試標 #[ignore]。

use super::*;
use chrono::NaiveDate;
use rust_decimal_macros::dec;

/// include_str! 的路徑相對於本檔案（tpex/capital_reduction/tests.rs）→ tpex/testdata/。
const FIXTURE: &str = include_str!("../testdata/capital_reduction_revivt.json");

fn parse_fixture() -> Vec<CorporateAction> {
    let response: RevivtResponse = serde_json::from_str(FIXTURE).expect("fixture 應可反序列化");
    parse_capital_reductions(response)
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
}

/// fixture 的 5 列會收斂成 3 筆：3710 重複一次，4529 尚未恢復交易。
#[test]
fn parse_capital_reductions_dedupes_and_skips_unresumed() {
    let actions = parse_fixture();

    assert_eq!(actions.len(), 3);
    assert!(
        actions.iter().all(|item| item.stock_symbol != "4529"),
        "價格為 - 的 4529 不應出現"
    );
    assert_eq!(
        actions
            .iter()
            .filter(|item| item.stock_symbol == "3710")
            .count(),
        1,
        "同一 (代號, 生效日) 只能保留一筆"
    );
}

/// 民國短日期 1150921 → 2026-09-21，比率 5.13 ÷ 8.32 ≈ 0.6166。
#[test]
fn parse_capital_reductions_parses_short_roc_date_and_ratio() {
    let actions = parse_fixture();
    let item = actions
        .iter()
        .find(|item| item.stock_symbol == "3710")
        .expect("fixture 應含 3710");

    assert_eq!(item.effective_date, date(2026, 9, 21));
    assert_eq!(item.action_type, CorporateActionType::CapitalReduction);
    assert_eq!(item.note, "減資彌補虧損");
    assert_eq!(item.share_ratio.round_dp(4), dec!(0.6166));
}

/// 8277 商丞：8.05 ÷ 18.67 ≈ 0.4312，驗證另一筆不同量級的比率。
#[test]
fn parse_capital_reductions_handles_large_reduction() {
    let actions = parse_fixture();
    let item = actions
        .iter()
        .find(|item| item.stock_symbol == "8277")
        .expect("fixture 應含 8277");

    assert_eq!(item.share_ratio.round_dp(4), dec!(0.4312));
}

/// 結果依 (代號, 生效日) 排序。
#[test]
fn parse_capital_reductions_is_sorted() {
    let actions = parse_fixture();
    let symbols: Vec<&str> = actions
        .iter()
        .map(|item| item.stock_symbol.as_str())
        .collect();
    let mut sorted = symbols.clone();
    sorted.sort_unstable();

    assert_eq!(symbols, sorted);
}

/// 沒有 `tables` 時應回空陣列而不是 panic。
#[test]
fn parse_capital_reductions_handles_empty_tables() {
    let response: RevivtResponse = serde_json::from_str(r#"{"date":"","tables":[]}"#).unwrap();

    assert!(parse_capital_reductions(response).is_empty());
}

#[tokio::test]
#[ignore = "需連線至 TPEx 網站"]
async fn test_visit() {
    dotenvy::dotenv().ok();

    match visit().await {
        Ok(actions) => {
            println!("取得 {} 筆上櫃減資事件", actions.len());
            for item in &actions {
                println!(
                    "  {} {} 比率 {} {}",
                    item.stock_symbol,
                    item.effective_date,
                    item.share_ratio.round_dp(6),
                    item.note
                );
            }
        }
        Err(why) => println!("visit 失敗: {why:?}"),
    }
}
