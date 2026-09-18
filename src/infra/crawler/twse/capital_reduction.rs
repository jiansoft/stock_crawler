//! # 上市股票減資恢復買賣參考價格
//!
//! 來源：TWSE `rwd/zh/reducation/TWTAUU`（股票減資恢復買賣參考價格）。
//!
//! ## 這個端點支援日期區間
//!
//! 參數名是 **`startDate` / `endDate`**（西元 `YYYYMMDD`），不是其他 TWSE 端點
//! 常見的 `strDate`。用錯名字時它不會忽略參數，而是回
//! `{"stat":"查詢結束日期小於查詢開始日期，請重新查詢!"}`，很容易被誤判成
//! 「此端點只回當日資料」。實測 `startDate=20150101&endDate=20261231`
//! 單一請求即可取回十餘年的全部事件，**不需要逐日輪詢**。
//!
//! ## 換股比率怎麼算
//!
//! 表格只給「停止買賣前收盤價格」與「恢復買賣參考價」，沒有每股退還現金。
//! TWSE 的參考價公式是：
//!
//! ```text
//! 恢復買賣參考價 = (停止買賣前收盤價 − 每股退還現金) ÷ 換股比率
//! ```
//!
//! 彌補虧損沒有現金退還，比率就是 `前收 ÷ 參考價`。退還股款則需要現金項，
//! 但若把退還的現金**在恢復日以參考價立即再投入**，可多買 `現金 ÷ 參考價` 股，
//! 總股數為：
//!
//! ```text
//! (前收 − 現金) ÷ 參考價 + 現金 ÷ 參考價 = 前收 ÷ 參考價
//! ```
//!
//! 兩種原因都收斂到同一個式子，且財富在事件前後連續。這與
//! [`crate::domain::performance::simulator`] 口徑 C（含息再投入）對現金股利的
//! 處理慣例一致，因此本模組一律以 `前收 ÷ 參考價` 作為
//! [`CorporateAction::share_ratio`]，不另外抓每股退還現金的明細端點。
//!
//! 副作用：退還股款的比率可能**大於 1**（參考價低於前收，例如 8201 無敵
//! 2016-07-18 的 1.0702）。這是正確的結果，不是資料錯誤，因此
//! `action_type` 一律明確指定為 [`CorporateActionType::CapitalReduction`]，
//! 不可由比例反推。
//!
//! ## 三個資料陷阱
//!
//! 1. **尚未恢復交易的列價格是 `-`**：公告已發布但換發尚未完成，這類列沒有
//!    參考價可算比率，必須略過而不是當成 0。
//! 2. **同一 `(代號, 恢復買賣日期)` 會有多列**：同一次減資可能多次公告
//!    （例如 3536 誠創 2015-03-20 有三列，公告日分別是 2015-01-07、
//!    2015-01-13、2015-03-31），以「詳細資料」欄夾帶的公告日取**最新**一筆。
//! 3. **恢復買賣當天可能同時除權**：此時「除權參考價」欄另有數值。比率的分母
//!    仍取「恢復買賣參考價」——除權造成的價格變動由
//!    [`crate::domain::performance::entity::DividendEvent`] 負責，
//!    在這裡一併計入會重複調整。

use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

use crate::{
    core::util,
    domain::performance::{CorporateAction, CorporateActionType},
    infra::crawler::twse,
};

/// TWTAUU 的回應主體。
///
/// 端點以 `fields` + `data` 的二維字串陣列回傳，欄位順序固定，
/// 因此解析時以索引取值（對照 [`FIELD_RESUMED_DATE`] 等常數）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TwtauuResponse {
    /// 查詢狀態，成功時為 `OK`。
    #[serde(default)]
    pub stat: String,
    /// 資料列；查無資料時端點不會給這個欄位。
    #[serde(default)]
    pub data: Option<Vec<Vec<String>>>,
}

/// 查詢成功時的 `stat` 值。
const STAT_OK: &str = "OK";

/// 「查詢區間內真的沒有減資」時 `stat` 會包含的字樣。
///
/// 端點用同一個 `stat` 欄位表達三種截然不同的狀態，且都不帶 `data`：
///
/// | 情況 | `stat` |
/// |------|--------|
/// | 成功 | `OK` |
/// | 區間內無事件 | `很抱歉，沒有符合條件的資料!` |
/// | 查詢被拒 | `查詢開始日期小於100年1月1日，請重新查詢!` 等 |
///
/// 前兩者是正常結果，最後一種必須當成錯誤往上拋——否則像「起始日早於
/// 民國 100 年」這種參數錯誤會被靜默吞掉，全量回補會「成功」寫入 0 筆。
const STAT_NO_DATA: &str = "沒有符合條件的資料";

/// 欄位索引：恢復買賣日期（民國 `YYY/MM/DD`）。
const FIELD_RESUMED_DATE: usize = 0;
/// 欄位索引：股票代號。
const FIELD_SYMBOL: usize = 1;
/// 欄位索引：停止買賣前收盤價格。
const FIELD_PREVIOUS_CLOSE: usize = 3;
/// 欄位索引：恢復買賣參考價。
const FIELD_REFERENCE_PRICE: usize = 4;
/// 欄位索引：減資原因（彌補虧損／退還股款）。
const FIELD_REASON: usize = 9;
/// 欄位索引：詳細資料，格式為 `代號  ,公告日`，例如 `3536  ,20150331`。
const FIELD_DETAIL: usize = 10;
/// 一列至少要有這麼多欄才可能解析成功。
const MIN_FIELD_COUNT: usize = FIELD_DETAIL + 1;

/// 查詢起始日的下限：民國 100 年 1 月 1 日。
///
/// 早於此日的查詢會被端點拒絕（`查詢開始日期小於100年1月1日，請重新查詢!`），
/// 且因為被拒時同樣不帶 `data`，若不特別區分就會靜默得到空結果。
pub const EARLIEST_QUERYABLE_DATE: (i32, u32, u32) = (2011, 1, 1);

/// 取得指定期間內全部上市股票的減資恢復買賣事件。
///
/// `start` 與 `end` 為西元日期，端點以「恢復買賣日期」落在區間內為條件回傳。
/// 區間可以橫跨十餘年，單一請求即可完成全量回補；`start` 不得早於
/// [`EARLIEST_QUERYABLE_DATE`]。
///
/// # 錯誤
///
/// 下列情況回傳錯誤：
///
/// - HTTP 請求或 JSON 反序列化失敗；
/// - 端點拒絕查詢（例如起始日早於 [`EARLIEST_QUERYABLE_DATE`]、
///   或結束日早於起始日）。
///
/// 區間內沒有任何減資則回傳空陣列，這是正常情況而非錯誤。
pub async fn visit(
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
) -> Result<Vec<CorporateAction>> {
    let url = format!(
        "https://www.{host}/rwd/zh/reducation/TWTAUU?response=json&startDate={start}&endDate={end}",
        host = twse::HOST,
        start = start.format("%Y%m%d"),
        end = end.format("%Y%m%d"),
    );

    let response = util::http::get_json::<TwtauuResponse>(&url).await?;

    ensure_queryable(&response.stat, start, end)?;

    Ok(parse_capital_reductions(response))
}

/// 檢查端點回報的狀態是否代表查詢本身被接受。
///
/// 把「查詢被拒」與「查無資料」分開：兩者都不帶 `data`，若一律視為空結果，
/// 參數錯誤就會被靜默吞掉，呼叫端會以為回補成功但其實一筆都沒寫。
fn ensure_queryable(stat: &str, start: chrono::NaiveDate, end: chrono::NaiveDate) -> Result<()> {
    if stat == STAT_OK || stat.contains(STAT_NO_DATA) {
        return Ok(());
    }

    anyhow::bail!(
        "TWSE 拒絕減資查詢（{start} ~ {end}）：{stat}。可查詢的最早起始日為民國 100 年 1 月 1 日"
    )
}

/// 將 TWTAUU 的原始回應整理成 [`CorporateAction`]。
///
/// 這是一個**純函式**——輸入只有已反序列化的原始資料，不做任何網路 I/O，
/// 可用 `testdata/capital_reduction_twtauu.json` fixture 直接驗證，涵蓋
/// 模組說明中的三個資料陷阱。
///
/// 回傳結果依 `(代號, 生效日)` 排序，且每個鍵只會出現一次。
fn parse_capital_reductions(response: TwtauuResponse) -> Vec<CorporateAction> {
    let Some(rows) = response.data else {
        return Vec::new();
    };

    // 以 (代號, 生效日) 為鍵去重，值另存公告日供「取最新」比較。
    // 公告日無法解析時以空字串參與比較，字典序上一定輸給任何實際日期，
    // 因此有公告日的那一列會勝出。
    let mut latest: HashMap<(String, chrono::NaiveDate), (String, CorporateAction)> =
        HashMap::new();

    for row in rows {
        let Some((announced_at, action)) = parse_row(&row) else {
            continue;
        };

        let key = (action.stock_symbol.clone(), action.effective_date);
        latest
            .entry(key)
            .and_modify(|existing| {
                if announced_at > existing.0 {
                    *existing = (announced_at.clone(), action.clone());
                }
            })
            .or_insert((announced_at, action));
    }

    let mut result: Vec<CorporateAction> = latest.into_values().map(|(_, action)| action).collect();
    result.sort_by(|a, b| {
        a.stock_symbol
            .cmp(&b.stock_symbol)
            .then(a.effective_date.cmp(&b.effective_date))
    });

    result
}

/// 解析單一資料列，回傳 `(公告日, 公司行動)`。
///
/// 無法解析或資料不完整時回傳 `None`——這在此端點是**正常情況**
/// （尚未恢復交易的列價格為 `-`），因此不記錄為錯誤。
fn parse_row(row: &[String]) -> Option<(String, CorporateAction)> {
    if row.len() < MIN_FIELD_COUNT {
        return None;
    }

    let effective_date = util::datetime::parse_taiwan_date(row[FIELD_RESUMED_DATE].trim())?;
    let stock_symbol = row[FIELD_SYMBOL].trim();
    if stock_symbol.is_empty() {
        return None;
    }

    // 陷阱 1：尚未恢復交易的列，價格欄是 `-`，parse_decimal 會失敗。
    let previous_close = util::text::parse_decimal(row[FIELD_PREVIOUS_CLOSE].trim(), None).ok()?;
    // 陷阱 3：分母固定取「恢復買賣參考價」，不取「除權參考價」。
    let reference_price =
        util::text::parse_decimal(row[FIELD_REFERENCE_PRICE].trim(), None).ok()?;

    // 兩者都必須為正：參考價為 0 無法相除，前收為 0 會讓持股歸零。
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

    Some((
        parse_announced_at(&row[FIELD_DETAIL]),
        CorporateAction {
            stock_symbol: stock_symbol.to_string(),
            effective_date,
            // 來源已明確知道這是減資，不可由比例反推：
            // 退還股款的比例可能大於 1，反推會誤判成分割。
            action_type: CorporateActionType::CapitalReduction,
            share_ratio,
            note,
        },
    ))
}

/// 自「詳細資料」欄取出公告日。
///
/// 欄位格式為 `代號  ,公告日`（例如 `3536  ,20150331`）。取不到時回傳空字串，
/// 讓它在「取最新公告」的比較中一定落敗。
fn parse_announced_at(detail: &str) -> String {
    detail
        .split(',')
        .nth(1)
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    /// include_str! 的路徑相對於本檔案（twse/capital_reduction.rs）→ twse/testdata/。
    const FIXTURE: &str = include_str!("testdata/capital_reduction_twtauu.json");

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
}
