//! TWSE 減資爬蟲的單元測試。

use super::*;
use chrono::NaiveDate;
use rust_decimal_macros::dec;

/// include_str! 的路徑相對於本檔案（twse/capital_reduction/tests.rs）→ twse/testdata/。
const FIXTURE: &str = include_str!("../testdata/capital_reduction_twtauu.json");

fn parse_fixture() -> Vec<CorporateAction> {
    let response: TwtauuResponse = serde_json::from_str(FIXTURE).expect("fixture 應可反序列化");
    parse_capital_reductions(response)
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("測試日期應合法")
}

fn find<'a>(actions: &'a [CorporateAction], symbol: &str) -> &'a CorporateAction {
    actions
        .iter()
        .find(|item| item.stock_symbol == symbol)
        .unwrap_or_else(|| panic!("fixture 應含 {symbol}"))
}

/// fixture 的 9 列會收斂成 5 筆：3536 的三列合併成一筆（3040、6165、8201、
/// 3312 各一筆），2601 與 2323 兩列因尚未恢復交易（價格為 `-`）被略過。
#[test]
fn parse_capital_reductions_dedupes_and_skips_unresumed() {
    let actions = parse_fixture();

    assert_eq!(actions.len(), 5, "去重並略過未恢復交易後應剩 5 筆");

    // 陷阱 1：尚未恢復交易的列不得產生資料。
    assert!(
        actions.iter().all(|item| item.stock_symbol != "2601"),
        "價格為 - 的 2601 不應出現"
    );
    assert!(
        actions.iter().all(|item| item.stock_symbol != "2323"),
        "價格為 - 的 2323 不應出現"
    );

    // 陷阱 2：3536 誠創 2015-03-20 在 fixture 中有三列公告，只能留一筆。
    let cheng_chuang: Vec<&CorporateAction> = actions
        .iter()
        .filter(|item| item.stock_symbol == "3536")
        .collect();
    assert_eq!(cheng_chuang.len(), 1, "同一 (代號, 生效日) 只能保留一筆");
    assert_eq!(cheng_chuang[0].effective_date, date(2015, 3, 20));
}

/// 民國日期轉換與比率計算：6.58 ÷ 13.33 ≈ 0.4936。
#[test]
fn parse_capital_reductions_computes_ratio_from_reference_price() {
    let actions = parse_fixture();
    let item = find(&actions, "3536");

    assert_eq!(item.effective_date, date(2015, 3, 20));
    assert_eq!(item.action_type, CorporateActionType::CapitalReduction);
    assert_eq!(item.note, "減資彌補虧損");
    // 6.58 / 13.33，取到小數第四位比較，避免除不盡的尾數。
    assert_eq!(item.share_ratio.round_dp(4), dec!(0.4936));
}

/// 陷阱 3：3312 弘憶股恢復買賣當天另有「除權參考價」8.34，
/// 但比率的分母必須是「恢復買賣參考價」8.48。
#[test]
fn parse_capital_reductions_ignores_ex_rights_reference_price() {
    let actions = parse_fixture();
    let item = find(&actions, "3312");

    // 6.54 / 8.48 = 0.7712；若誤用除權參考價 8.34 會得到 0.7842。
    assert_eq!(item.share_ratio.round_dp(4), dec!(0.7712));
    assert_ne!(
        item.share_ratio.round_dp(4),
        dec!(0.7842),
        "分母不得取除權參考價"
    );
}

/// 退還股款的比率可能大於 1，且**必須**仍被標記為減資。
/// 這正是不能由比例反推 action_type 的理由。
#[test]
fn parse_capital_reductions_keeps_type_when_ratio_exceeds_one() {
    let actions = parse_fixture();
    let item = find(&actions, "8201");

    // 8.69 / 8.12 = 1.0702，大於 1。
    assert!(item.share_ratio > dec!(1), "退還股款的比率可能大於 1");
    assert_eq!(item.share_ratio.round_dp(4), dec!(1.0702));
    assert_eq!(
        item.action_type,
        CorporateActionType::CapitalReduction,
        "比率大於 1 仍是減資，不得被判成分割"
    );
    assert_eq!(item.note, "減資退還股款");
}

/// 結果依 (代號, 生效日) 排序，方便下游比對與記錄。
#[test]
fn parse_capital_reductions_is_sorted_by_symbol_and_date() {
    let actions = parse_fixture();
    let symbols: Vec<&str> = actions
        .iter()
        .map(|item| item.stock_symbol.as_str())
        .collect();
    let mut sorted = symbols.clone();
    sorted.sort_unstable();

    assert_eq!(symbols, sorted);
}

/// 查無資料時端點不給 `data` 欄位，應回空陣列而不是 panic。
#[test]
fn parse_capital_reductions_handles_missing_data() {
    let response: TwtauuResponse =
        serde_json::from_str(r#"{"stat":"查詢日期大於今日，請重新查詢!"}"#).unwrap();

    assert!(parse_capital_reductions(response).is_empty());
}

/// 欄位數不足的畸形列要被略過，不得讓整批解析失敗。
#[test]
fn parse_capital_reductions_skips_malformed_rows() {
    let response: TwtauuResponse =
        serde_json::from_str(r#"{"stat":"OK","data":[["104/01/23","3040"]]}"#).unwrap();

    assert!(parse_capital_reductions(response).is_empty());
}

/// 三種 `stat` 必須被區分開：只有「查詢被拒」才是錯誤。
#[test]
fn ensure_queryable_separates_rejection_from_empty_result() {
    let start = date(2011, 1, 1);
    let end = date(2026, 12, 31);

    assert!(ensure_queryable("OK", start, end).is_ok());
    assert!(
        ensure_queryable("很抱歉，沒有符合條件的資料!", start, end).is_ok(),
        "區間內沒有減資是正常結果，不是錯誤"
    );

    // 起始日早於民國 100 年——實際踩過的坑，必須往上拋而不是靜默回空。
    let rejected = ensure_queryable("查詢開始日期小於100年1月1日，請重新查詢!", start, end);
    assert!(rejected.is_err());
    let message = format!("{:#}", rejected.expect_err("已斷言為錯誤"));
    assert!(
        message.contains("TWSE 拒絕減資查詢"),
        "錯誤訊息應指出查詢被拒：{message}"
    );

    assert!(ensure_queryable("查詢結束日期小於查詢開始日期，請重新查詢!", start, end).is_err());
}

/// 端點下限就是民國 100 年 1 月 1 日。
#[test]
fn earliest_queryable_date_is_roc_100() {
    assert_eq!(EARLIEST_QUERYABLE_DATE, (2011, 1, 1));
}

#[test]
fn parse_announced_at_extracts_date() {
    assert_eq!(parse_announced_at("3536  ,20150331"), "20150331");
    assert_eq!(parse_announced_at("3536"), "");
    assert_eq!(parse_announced_at(""), "");
}

#[tokio::test]
#[ignore = "需連線至 TWSE 網站"]
async fn test_visit() {
    dotenvy::dotenv().ok();

    let start = date(2015, 1, 1);
    let end = date(2026, 12, 31);
    match visit(start, end).await {
        Ok(actions) => {
            println!("取得 {} 筆上市減資事件", actions.len());
            for item in actions.iter().take(5) {
                println!(
                    "  {} {} 比率 {} {}",
                    item.stock_symbol,
                    item.effective_date,
                    item.share_ratio.round_dp(6),
                    item.note
                );
            }
            assert!(!actions.is_empty(), "十年區間內應有減資事件");
        }
        Err(why) => println!("visit 失敗: {why:?}"),
    }
}
