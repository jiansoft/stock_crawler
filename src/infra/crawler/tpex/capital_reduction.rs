//! # 上櫃股票減資恢復買賣參考價格
//!
//! 來源：TPEx `www/zh-tw/bulletin/revivt`（減資恢復買賣參考價格公告）。
//!
//! ## 這個端點只有當週，沒有歷史
//!
//! 與上市的 [`crate::infra::crawler::twse::capital_reduction`] 不同，本端點
//! **不接受任何日期參數**：傳 `date`、`startDate`／`endDate` 一律被忽略或回
//! `{"stat":"參數錯誤"}`，回應固定是當週（回應中的 `date` 欄形如
//! `20260917~20260921`）。TPEx OpenAPI 亦無對應的歷史端點
//! （`tpex_spendi_history` 只有當年度且不含價格與減資原因）。
//!
//! 因此上櫃減資**必須靠每日排程累積**，漏掉的那一週就補不回來了；
//! 歷史缺口需另循他法回補。
//!
//! ## 換股比率與資料陷阱
//!
//! 比率的計算方式、以及「未恢復交易」「同鍵多列」「恢復日同時除權」三個陷阱的
//! 處理原則，與上市版完全相同，詳見
//! [`crate::infra::crawler::twse::capital_reduction`] 的模組說明。
//!
//! 兩處格式差異：
//!
//! - 日期是**無分隔的民國短格式**（`1150921`），不是上市版的 `115/09/21`。
//! - 「除權參考價」欄無值時是 `0.00` 而不是 `--`；本模組不使用這一欄，
//!   所以兩種寫法都不影響結果。

use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

use crate::{
    core::util,
    domain::performance::{CorporateAction, CorporateActionType},
    infra::crawler::tpex,
};

/// `revivt` 的回應主體。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RevivtResponse {
    /// 本次公告涵蓋的日期區間，形如 `20260917~20260921`。
    #[serde(default)]
    pub date: String,
    /// 資料表；查無資料時可能是空陣列。
    #[serde(default)]
    pub tables: Vec<RevivtTable>,
}

/// `revivt` 回應中的單一資料表。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RevivtTable {
    /// 資料列，欄位順序與 `fields` 對應。
    #[serde(default)]
    pub data: Vec<Vec<String>>,
}

/// 欄位索引：恢復買賣日期（民國短格式 `YYYMMDD`）。
const FIELD_RESUMED_DATE: usize = 0;
/// 欄位索引：股票代號。
const FIELD_SYMBOL: usize = 1;
/// 欄位索引：最後交易日之收盤價格。
const FIELD_PREVIOUS_CLOSE: usize = 3;
/// 欄位索引：減資恢復買賣開始日參考價格。
const FIELD_REFERENCE_PRICE: usize = 4;
/// 欄位索引：減資原因（彌補虧損／退還股款）。
const FIELD_REASON: usize = 9;
/// 一列至少要有這麼多欄才可能解析成功。
const MIN_FIELD_COUNT: usize = FIELD_REASON + 1;

/// 取得當期公告中的全部上櫃減資恢復買賣事件。
///
/// 端點沒有日期參數，回傳的一律是當週公告。
///
/// # 錯誤
///
/// 當 HTTP 請求或 JSON 反序列化失敗時回傳錯誤。當週沒有任何減資是常態，
/// 此時回傳空陣列而非錯誤。
pub async fn visit() -> Result<Vec<CorporateAction>> {
    let url = format!(
        "https://{host}/www/zh-tw/bulletin/revivt?response=json",
        host = tpex::HOST
    );

    let response = util::http::get_json::<RevivtResponse>(&url).await?;

    Ok(parse_capital_reductions(response))
}

/// 將 `revivt` 的原始回應整理成 [`CorporateAction`]。
///
/// 這是一個**純函式**——輸入只有已反序列化的原始資料，不做任何網路 I/O，
/// 可用 `testdata/capital_reduction_revivt.json` fixture 直接驗證。
///
/// 回傳結果依 `(代號, 生效日)` 排序，且每個鍵只會出現一次。
fn parse_capital_reductions(response: RevivtResponse) -> Vec<CorporateAction> {
    let mut latest: HashMap<(String, chrono::NaiveDate), CorporateAction> = HashMap::new();

    for table in response.tables {
        for row in table.data {
            let Some(action) = parse_row(&row) else {
                continue;
            };

            // 與上市版不同，本端點的「詳細資料」欄是一段 HTML，不含公告日，
            // 無從判斷新舊。同一週內同鍵重複出現極罕見，一律以**後出現者**為準。
            latest.insert((action.stock_symbol.clone(), action.effective_date), action);
        }
    }

    let mut result: Vec<CorporateAction> = latest.into_values().collect();
    result.sort_by(|a, b| {
        a.stock_symbol
            .cmp(&b.stock_symbol)
            .then(a.effective_date.cmp(&b.effective_date))
    });

    result
}

/// 解析單一資料列。
///
/// 無法解析或資料不完整時回傳 `None`——尚未恢復交易的列價格是 `-`，
/// 這是**正常情況**，不記錄為錯誤。
fn parse_row(row: &[String]) -> Option<CorporateAction> {
    if row.len() < MIN_FIELD_COUNT {
        return None;
    }

    // 上櫃用無分隔的民國短格式（1150921），與上市版的 115/09/21 不同。
    let effective_date = util::datetime::parse_taiwan_date_short(row[FIELD_RESUMED_DATE].trim())?;
    let stock_symbol = row[FIELD_SYMBOL].trim();
    if stock_symbol.is_empty() {
        return None;
    }

    let previous_close = util::text::parse_decimal(row[FIELD_PREVIOUS_CLOSE].trim(), None).ok()?;
    let reference_price =
        util::text::parse_decimal(row[FIELD_REFERENCE_PRICE].trim(), None).ok()?;

    if previous_close <= rust_decimal::Decimal::ZERO
        || reference_price <= rust_decimal::Decimal::ZERO
    {
        return None;
    }

    let share_ratio = previous_close.checked_div(reference_price)?;
    let reason = row[FIELD_REASON].trim();
    let note = if reason.is_empty() {
        "減資".to_string()
    } else {
        format!("減資{reason}")
    };

    Some(CorporateAction {
        stock_symbol: stock_symbol.to_string(),
        effective_date,
        // 來源已明確知道這是減資，不可由比例反推。
        action_type: CorporateActionType::CapitalReduction,
        share_ratio,
        note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    /// include_str! 的路徑相對於本檔案（tpex/capital_reduction.rs）→ tpex/testdata/。
    const FIXTURE: &str = include_str!("testdata/capital_reduction_revivt.json");

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
}
