use anyhow::{Result, anyhow};
use scraper::{Html, Selector};

use crate::{
    core::util::{self, convert::FromValue},
    infra::crawler::{share::QfiiDto, twse},
};

/// 取得上櫃股票外資及陸資投資持股統計
pub async fn visit() -> Result<Vec<QfiiDto>> {
    let url = format!(
        "https://mops.{}/server-java/t13sa150_otc?&step=wh",
        twse::HOST,
    );
    // 上櫃 QFII 頁是 Big5 編碼的舊式 HTML，先由 http helper 解碼成 UTF-8，
    // 解析交給純函式（fetch/parse 分離，解析邏輯才能被 fixture 單元測試覆蓋）。
    let text = util::http::get_use_big5(&url).await?;
    parse_otc_qfii_html(&text)
}

/// 解析 MOPS 上櫃外資持股統計頁（`t13sa150_otc`）的 HTML。
///
/// 這是一個「純函式」——輸入只有（已解碼成 UTF-8 的）HTML 字串，
/// 不做任何網路 I/O，可用 `testdata/qfii_otc.html` fixture 直接驗證。
///
/// # 解析規則（此頁是沒有 class/id 的舊式表格）
/// - 目標是 `body > center` 下第一個 `<table>` 的所有資料列。
/// - 每列以「文字節點」計數必須恰好 23 個——注意不是 `<td>` 數：
///   `node.text()` 會把一列裡所有文字節點依序攤平，
///   表頭與備註列的節點數不同，自然被略過。
/// - 文字節點索引：`[1]`＝股票代號、`[5]`＝發行股數、
///   `[9]`＝外資持有股數、`[13]`＝持股比率（可能帶 `&nbsp;`，需去除）。
fn parse_otc_qfii_html(text: &str) -> Result<Vec<QfiiDto>> {
    let selector = Selector::parse("body > center > table:nth-child(1) > tbody > tr")
        .map_err(|why| anyhow!("Failed to Selector::parse because: {:?}", why))?;
    let document = Html::parse_document(text);
    let mut result = Vec::with_capacity(1024);

    for node in document.select(&selector) {
        let tds: Vec<String> = node.text().map(|v| v.to_string()).collect();
        if tds.len() != 23 {
            continue;
        }
        let stock_symbol = tds[1].get_string(None);
        let issued_share = tds[5].get_i64(None);
        let shares_held = tds[9].get_i64(None);
        let share_holding_percentage = tds[13].get_decimal(Some(vec!['\u{a0}']));

        result.push(QfiiDto {
            stock_symbol,
            issued_share,
            shares_held,
            share_holding_percentage,
        });
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::result::Result::Ok;

    use crate::infra::cache::SHARE;

    use super::*;
    use rust_decimal_macros::dec;

    /// 以貼近 t13sa150_otc 真實頁面形狀的 fixture 驗證整頁解析流程。
    ///
    /// 這一頁的解析靠「每列文字節點恰好 23 個」定位資料列，
    /// fixture 同時驗證：表頭與合計列被略過、千分位逗號與 `&nbsp;` 被清除、
    /// 以及 center 下的第二個 table 不會被納入。
    #[test]
    fn parse_otc_qfii_html_parses_fixture_rows() {
        // include_str! 的路徑相對於本檔案（qualified_foreign_institutional_investor/
        // over_the_counter.rs）→ 同目錄的 testdata/。
        const FIXTURE: &str = include_str!("testdata/qfii_otc.html");

        let result = parse_otc_qfii_html(FIXTURE).unwrap();

        // 有效列只有兩筆：表頭列、合計列與第二個 table 都不該進結果。
        assert_eq!(result.len(), 2);
        assert!(!result.iter().any(|dto| dto.stock_symbol == "9999"));

        assert_eq!(result[0].stock_symbol, "5274");
        assert_eq!(result[0].issued_share, 36_915_132);
        assert_eq!(result[0].shares_held, 12_345_678);
        // 持股比率帶 &nbsp;（U+00A0）前綴，解析時要一併清除。
        assert_eq!(result[0].share_holding_percentage, dec!(33.44));

        assert_eq!(result[1].stock_symbol, "6488");
        assert_eq!(result[1].share_holding_percentage, dec!(22.69));
    }

    /// 與目標結構無關的頁面應回傳空清單，不 panic。
    #[test]
    fn parse_otc_qfii_html_returns_empty_for_unrelated_page() {
        let result = parse_otc_qfii_html("<html><body><p>系統維護中</p></body></html>").unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 visit");

        match visit().await {
            Ok(qfiis) => {
                dbg!(&qfiis);
                tracing::debug!("qfiis:{:#?}", qfiis);
            }
            Err(why) => {
                tracing::debug!("Failed to visit because: {:?}", why);
            }
        }
        tracing::debug!("結束 visit");
    }
}
