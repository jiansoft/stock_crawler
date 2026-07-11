//! 公開資訊觀測站季 EPS 爬蟲。
//!
//! 此模組負責向公開資訊觀測站請求指定市場、年度與季度的季 EPS 清單，
//! 並解析成 [`Eps`] 結構。

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use rust_decimal::Decimal;
use scraper::{Html, Selector};

use crate::{
    core::declare::{Quarter, StockExchangeMarket},
    core::util::{self, convert::FromValue, datetime},
    infra::cache::SHARE,
    infra::crawler::twse,
};

#[derive(Debug, Clone)]
/// 單一股票於指定年度與季度的 EPS 資料。
pub struct Eps {
    /// 年度
    pub year: i32,
    /// 季度 Q4 Q3 Q2 Q1
    pub quarter: Quarter,
    /// 股票代號。
    pub stock_symbol: String,
    /// 每股稅後淨利
    pub earnings_per_share: Decimal,
}

impl Eps {
    /// 建立一筆季 EPS 資料。
    pub fn new(stock_symbol: String, year: i32, quarter: Quarter, eps: Decimal) -> Self {
        Self {
            year,
            quarter,
            stock_symbol,
            earnings_per_share: eps,
        }
    }
}

/// 向公開資訊觀測站抓取指定市場與季度的 EPS 清單。
///
/// # 參數
///
/// * `stock_exchange_market` - 市場別，例如上市或上櫃
/// * `year` - 目標財報年度（西元年）
/// * `quarter` - 目標財報季度
///
/// # 回傳值
///
/// 成功時回傳符合條件的 [`Eps`] 清單；失敗時回傳錯誤。
///
/// # 錯誤
///
/// 當 HTTP 請求失敗、回應無法解析，或來源站結構異常時回傳錯誤。
pub async fn visit(
    stock_exchange_market: StockExchangeMarket,
    year: i32,
    quarter: Quarter,
) -> Result<Vec<Eps>> {
    let url = format!(
        "https://mopsov.{host}/mops/web/ajax_t163sb19",
        host = twse::HOST,
    );
    let roc_year = datetime::gregorian_year_to_roc_year(year).to_string();
    let season = format!("0{season}", season = quarter.serial());
    let typek = match stock_exchange_market {
        StockExchangeMarket::Public => "pub",
        StockExchangeMarket::Listed => "sii",
        StockExchangeMarket::OverTheCounter => "otc",
        StockExchangeMarket::Emerging => "rotc",
    };
    let mut params = HashMap::with_capacity(7);
    params.insert("encodeURIComponent", "1");
    params.insert("step", "1");
    params.insert("firstin", "1");
    params.insert("year", &roc_year);
    params.insert("season", &season);
    params.insert("code", "");
    params.insert("TYPEK", typek);

    let response = util::http::post(&url, None, Some(params))
        .await
        .map_err(|err| anyhow!("HTTP request failed: {}", err))?;

    // 解析交給純函式（fetch/parse 分離，解析邏輯才能被 fixture 單元測試覆蓋）。
    // 「代號是否為已知股票」的判斷以閉包注入：正式流程查 SHARE 快取，
    // 測試則給一個固定清單，讓解析測試不需要載入全域快取。
    Ok(parse_eps_html(&response, year, quarter, |symbol| {
        SHARE.stock_contains_key(symbol)
    }))
}

/// 解析公開資訊觀測站（MOPS）季 EPS 頁面的 HTML。
///
/// 這是一個「純函式」——輸入只有 HTML 字串與判斷條件，不做任何網路 I/O，
/// 可用 `testdata/eps_t163sb19.html` fixture 直接驗證。
///
/// # 解析規則
/// - MOPS 回應中可能有多個 `<table>`（各產業一表），全部掃描。
/// - 只吃「恰好 9 個 `<td>`」的列：表頭、產業小標與備註列自然被略過。
/// - `tds[0]`＝股票代號、`tds[3]`＝每股稅後淨利（EPS）；
///   EPS 無法解析時以 0 落地（來源偶爾出現 `N/A` 或空欄）。
/// - `is_known_symbol` 過濾非追蹤中的代號（例如已下市或非股票類）。
fn parse_eps_html(
    html: &str,
    year: i32,
    quarter: Quarter,
    is_known_symbol: impl Fn(&str) -> bool,
) -> Vec<Eps> {
    let document = Html::parse_document(html);
    let mut result = Vec::with_capacity(1024);
    let selector_table = Selector::parse("table").expect("Failed to parse table selector");
    let selector_tr = Selector::parse("tr").expect("Failed to parse tr selector");
    let selector_td = Selector::parse("td").expect("Failed to parse td selector");
    for table in document.select(&selector_table) {
        for tr in table.select(&selector_tr) {
            let tds: Vec<_> = tr
                .select(&selector_td)
                .map(|td| td.text().collect::<String>().trim().to_string())
                .collect();

            if tds.len() != 9 {
                continue;
            }

            let stock_symbol = &tds[0];

            if stock_symbol.is_empty() {
                continue;
            }

            if !is_known_symbol(stock_symbol) {
                continue;
            }

            let eps = Eps::new(
                stock_symbol.to_string(),
                year,
                quarter,
                tds[3].to_string().get_decimal(None),
            );

            result.push(eps);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use crate::infra::cache::SHARE;

    use super::*;
    use rust_decimal_macros::dec;

    /// 以貼近 MOPS 真實回應形狀的 fixture 驗證整頁解析流程。
    ///
    /// `is_known_symbol` 用固定清單注入，不依賴 SHARE 全域快取——
    /// 這正是把它做成閉包參數的目的：解析測試完全離線、無外部狀態。
    #[test]
    fn parse_eps_html_parses_fixture_rows() {
        // include_str! 的路徑相對於本檔案（twse/eps.rs）→ twse/testdata/。
        const FIXTURE: &str = include_str!("testdata/eps_t163sb19.html");
        let known = ["2330", "2317"];

        let result = parse_eps_html(FIXTURE, 2025, Quarter::Q3, |symbol| known.contains(&symbol));

        // 有效列只有 2330 與 2317：表頭、備註、空代號與非追蹤代號（9998）都被略過。
        assert_eq!(result.len(), 2);

        assert_eq!(result[0].stock_symbol, "2330");
        assert_eq!(result[0].year, 2025);
        assert_eq!(result[0].quarter, Quarter::Q3);
        assert_eq!(result[0].earnings_per_share, dec!(15.36));

        // EPS 欄為「N/A」時以 0 落地，而不是讓整頁解析失敗。
        assert_eq!(result[1].stock_symbol, "2317");
        assert_eq!(result[1].earnings_per_share, Decimal::ZERO);
    }

    /// 與目標結構無關的頁面（維護頁、錯誤頁）應回傳空清單，不 panic。
    #[test]
    fn parse_eps_html_returns_empty_for_unrelated_page() {
        let result = parse_eps_html(
            "<html><body><p>查無資料</p></body></html>",
            2025,
            Quarter::Q3,
            |_| true,
        );
        assert!(result.is_empty());
    }

    #[tokio::test]
    #[ignore = "live test：連線真實外部網站，需要時手動執行"]
    async fn test_visit() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 visit");

        match visit(StockExchangeMarket::Listed, 2025, Quarter::Q4).await {
            Ok(list) => {
                dbg!(&list);
                tracing::debug!("list:{:#?}", list);
            }
            Err(why) => {
                tracing::debug!("Failed to visit because: {:?}", why);
            }
        }

        tracing::debug!("結束 visit");
    }
}
