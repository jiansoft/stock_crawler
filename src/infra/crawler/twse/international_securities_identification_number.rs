use anyhow::Result;
use chrono::Local;
use scraper::{Html, Selector};

use crate::{
    core::declare::StockExchangeMarket,
    core::util::{self, datetime::Weekend},
    infra::crawler::twse,
};

const REQUIRED_CATEGORIES: [&str; 4] = ["股票", "特別股", "普通股", "臺灣存託憑證(TDR)"];

/// twse 國際證券識別碼
#[derive(Debug, Clone)]
pub struct InternationalSecuritiesIdentificationNumber {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 國際證券識別碼（ISIN）。
    pub isin_code: String,
    /// 上市日期。
    pub listing_date: String,
    /// 產業分類名稱。
    pub industry: String,
    /// CFI Code。
    pub cfi_code: String,
    /// 交易市場。
    pub market: StockExchangeMarket,
}

/// 調用  twse API 取得台股國際證券識別碼
/// 上市:2 上櫃︰4 興櫃︰5
pub async fn visit(
    mode: StockExchangeMarket,
) -> Result<Vec<InternationalSecuritiesIdentificationNumber>> {
    if Local::now().is_weekend() {
        return Ok(Vec::new());
    }

    let url = format!(
        "https://isin.{}/isin/C_public.jsp?strMode={}",
        twse::HOST,
        mode.serial()
    );

    let response = util::http::get_use_big5(&url).await?;
    Ok(parse_isin_html(&response, mode))
}

/// 解析 TWSE ISIN HTML 頁面，回傳符合條件的識別碼列表。
///
/// 只保留 `REQUIRED_CATEGORIES` 中的證券類別（股票、特別股、普通股、TDR）。
/// 每列第一欄格式為「代號\u{3000}名稱」（全形空格分隔）；欄位數 5 表示省略產業分類。
fn parse_isin_html(
    html: &str,
    mode: StockExchangeMarket,
) -> Vec<InternationalSecuritiesIdentificationNumber> {
    let mut result: Vec<InternationalSecuritiesIdentificationNumber> = Vec::with_capacity(4096);
    let document = Html::parse_document(html);

    let Ok(selector) = Selector::parse("body > table.h4 > tbody > tr") else {
        return result;
    };

    let mut is_required_category = false;
    for node in document.select(&selector).skip(1) {
        let tds: Vec<&str> = node.text().map(str::trim).collect();
        if tds.len() == 2 {
            is_required_category = REQUIRED_CATEGORIES.contains(&tds[0].trim());
            continue;
        }

        if !is_required_category {
            continue;
        }

        let split: Vec<&str> = tds[0].split('\u{3000}').collect();
        if split.len() != 2 {
            continue;
        }

        let (industry, cfi_code) = match tds.len() {
            5 => ("未分類".to_string(), tds[4].to_owned()),
            6 => (tds[4].to_owned(), tds[5].to_owned()),
            _ => ("未分類".to_string(), String::new()),
        };

        result.push(InternationalSecuritiesIdentificationNumber {
            stock_symbol: split[0].trim().to_owned(),
            name: split[1].to_owned(),
            isin_code: tds[1].to_owned(),
            listing_date: tds[2].to_owned(),
            industry,
            cfi_code,
            market: mode,
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小 HTML fixture：含一個類別標頭列與兩筆資料列（6 欄、5 欄各一）。
    const FIXTURE_HTML: &str = r#"<html><body>
<table class="h4"><tbody>
<tr><th>證券名稱及代號</th><th>國際證券辨識號碼(ISIN Code)</th><th>上市日</th><th>市場別</th><th>產業別</th><th>CFI Code</th></tr>
<tr><td>股票</td><td>類別</td></tr>
<tr><td>2330　台積電</td><td>TW0002330008</td><td>1994/09/05</td><td>上市</td><td>半導體業</td><td>ESVUFR</td></tr>
<tr><td>2317　鴻海</td><td>TW0002317006</td><td>1991/06/11</td><td>上市</td><td>電腦及週邊設備業</td><td>ESVUFR</td></tr>
<tr><td>特別股</td><td>類別</td></tr>
<tr><td>2330A　台積電甲特</td><td>TW0002330016</td><td>2022/01/01</td><td>上市</td><td>ESVPFR</td></tr>
</tbody></table>
</body></html>"#;

    #[test]
    fn test_parse_isin_html_basic() {
        let result = parse_isin_html(FIXTURE_HTML, StockExchangeMarket::Listed);
        // 2 股票 + 1 特別股（5 欄，industry = 未分類）
        assert_eq!(result.len(), 3);

        let tsmc = &result[0];
        assert_eq!(tsmc.stock_symbol, "2330");
        assert_eq!(tsmc.name, "台積電");
        assert_eq!(tsmc.isin_code, "TW0002330008");
        assert_eq!(tsmc.listing_date, "1994/09/05");
        assert_eq!(tsmc.industry, "半導體業");
        assert_eq!(tsmc.cfi_code, "ESVUFR");

        // 5 欄版本：industry 應填 "未分類"
        let preferred = &result[2];
        assert_eq!(preferred.stock_symbol, "2330A");
        assert_eq!(preferred.industry, "未分類");
        assert_eq!(preferred.cfi_code, "ESVPFR");
    }

    #[test]
    fn test_parse_isin_html_skips_non_required_category() {
        let html = r#"<html><body><table class="h4"><tbody>
<tr><th>header</th></tr>
<tr><td>認購權證</td><td>類別</td></tr>
<tr><td>9999　某權證</td><td>TW1234567890</td><td>2020/01/01</td><td>上市</td><td>其他</td><td>RWXXXX</td></tr>
</tbody></table></body></html>"#;
        let result = parse_isin_html(html, StockExchangeMarket::Listed);
        assert!(result.is_empty(), "認購權證不在 REQUIRED_CATEGORIES 中，應被過濾");
    }

    #[test]
    fn test_parse_isin_html_empty_table() {
        let html = r#"<html><body><table class="h4"><tbody></tbody></table></body></html>"#;
        let result = parse_isin_html(html, StockExchangeMarket::Listed);
        assert!(result.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn test_visit() {
        dotenv::dotenv().ok();
        tracing::debug!("開始 visit");
        for mode in StockExchangeMarket::iterator() {
            match visit(mode).await {
                Err(why) => {
                    tracing::error!("Failed to visit because {:?}", why);
                }
                Ok(result) => {
                    for item in result.iter() {
                        tracing::debug!("item:{:#?}", item);
                    }
                }
            }
        }
        tracing::debug!("結束 visit");
    }
}
