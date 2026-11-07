//! TWSE ISIN 名冊頁的解析與抓取測試。

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
<tr><td>臺灣存託憑證(TDR)</td><td>類別</td></tr>
<tr><td>9103　美德醫療-DR</td><td>TW0009103002</td><td>2002/12/13</td><td>上市</td><td>EDSDDR</td></tr>
</tbody></table>
</body></html>"#;

/// 全類別名冊解析：不過濾類別，ETF 等也要納入；類別標頭列跳過。
#[test]
fn parse_all_symbols_html_includes_every_category() {
    let html = r#"<html><body>
<table class="h4"><tbody>
<tr><th>證券名稱及代號</th><th>ISIN</th><th>上市日</th><th>市場別</th><th>產業別</th><th>CFI Code</th></tr>
<tr><td>股票</td><td>類別</td></tr>
<tr><td>2330　台積電</td><td>TW0002330008</td><td>1994/09/05</td><td>上市</td><td>半導體業</td><td>ESVUFR</td></tr>
<tr><td>ETF</td><td>類別</td></tr>
<tr><td>0050　元大台灣50</td><td>TW0000050004</td><td>2003/06/30</td><td>上市</td><td>CEOGEU</td></tr>
<tr><td>00699R　元大美債7-10反1</td><td>TW00000699R5</td><td>2017/06/08</td><td>上市</td><td>CEOMRU</td></tr>
<tr><td>受益證券</td><td>類別</td></tr>
<tr><td>01001T　土銀富邦R1</td><td>TW00001001T8</td><td>2005/03/10</td><td>上市</td><td>CBCIXU</td></tr>
</tbody></table>
</body></html>"#;

    let symbols = parse_all_symbols_html(html);

    // 股票、ETF（含反向）、受益證券全部都要在名冊內。
    assert_eq!(symbols.len(), 4);
    assert!(symbols.contains("2330"));
    assert!(symbols.contains("0050"));
    assert!(symbols.contains("00699R"));
    assert!(symbols.contains("01001T"));
}

/// 與目標結構無關的頁面回傳空集合，交由呼叫端的名冊大小安全閥攔下。
#[test]
fn parse_all_symbols_html_returns_empty_for_unrelated_page() {
    assert!(parse_all_symbols_html("<html><body><p>維護中</p></body></html>").is_empty());
}

#[test]
fn test_parse_isin_html_basic() {
    let result = parse_isin_html(FIXTURE_HTML, StockExchangeMarket::Listed);
    // 2 股票 + 1 特別股（5 欄，industry = 未分類）+ 1 TDR（5 欄，industry = 存託憑證）
    assert_eq!(result.len(), 4);

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

    let tdr = &result[3];
    assert_eq!(tdr.stock_symbol, "9103");
    assert_eq!(tdr.industry, "存託憑證");
    assert_eq!(tdr.cfi_code, "EDSDDR");
}

#[test]
fn test_parse_isin_html_skips_non_required_category() {
    let html = r#"<html><body><table class="h4"><tbody>
<tr><th>header</th></tr>
<tr><td>認購權證</td><td>類別</td></tr>
<tr><td>9999　某權證</td><td>TW1234567890</td><td>2020/01/01</td><td>上市</td><td>其他</td><td>RWXXXX</td></tr>
</tbody></table></body></html>"#;
    let result = parse_isin_html(html, StockExchangeMarket::Listed);
    assert!(
        result.is_empty(),
        "認購權證不在 REQUIRED_CATEGORIES 中，應被過濾"
    );
}

#[test]
fn test_parse_isin_html_empty_table() {
    let html = r#"<html><body><table class="h4"><tbody></tbody></table></body></html>"#;
    let result = parse_isin_html(html, StockExchangeMarket::Listed);
    assert!(result.is_empty());
}

/// 以本機 mock server 取代 isin.twse.com.tw：頁面是 Big5，抓取後依類別解析。
mod mock_source {
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::core::util::http::mock_origin::with_origins;

    const ISIN: &str = "https://isin.twse.com.tw";

    async fn listed_page() -> MockServer {
        let (body, _, _) = encoding_rs::BIG5.encode(FIXTURE_HTML);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/isin/C_public.jsp"))
            .and(query_param("strMode", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.into_owned()))
            .mount(&server)
            .await;
        server
    }

    /// 上市名冊（strMode=2）以 Big5 解碼後解析出股票、特別股與 TDR。
    #[tokio::test]
    async fn fetch_page_decodes_big5_listing() {
        let server = listed_page().await;
        let uri = server.uri();

        let html = with_origins(&[(ISIN, &uri)], fetch_page(StockExchangeMarket::Listed))
            .await
            .expect("mock 頁面");
        let items = parse_isin_html(&html, StockExchangeMarket::Listed);

        let symbols: Vec<&str> = items
            .iter()
            .map(|item| item.stock_symbol.as_str())
            .collect();
        assert_eq!(symbols, ["2330", "2317", "2330A", "9103"]);
        assert_eq!(items[0].name, "台積電");
    }

    /// 兩個排程入口週末一律回空（來源週末不更新）；平日改抓 mock 頁面。
    #[tokio::test]
    async fn visit_entries_skip_weekends_or_parse_the_page() {
        let server = listed_page().await;
        let uri = server.uri();
        let weekend = Local::now().is_weekend();

        let (items, symbols) = with_origins(&[(ISIN, &uri)], async {
            (
                visit(StockExchangeMarket::Listed).await.expect("visit"),
                visit_all_listed_symbols(StockExchangeMarket::Listed)
                    .await
                    .expect("visit_all_listed_symbols"),
            )
        })
        .await;

        let expected = if weekend { 0 } else { 4 };
        assert_eq!((items.len(), symbols.len()), (expected, expected));
    }

    /// 兩個入口的本體：平日抓 mock 頁面解析，週末不打來源直接回空。
    #[tokio::test]
    async fn entries_parse_on_weekdays_and_skip_weekends() {
        use chrono::TimeZone;

        let server = listed_page().await;
        let uri = server.uri();
        // 1999-03-10 是星期三、03-13 是星期六。
        let weekday = Local
            .with_ymd_and_hms(1999, 3, 10, 5, 0, 0)
            .single()
            .expect("固定時間合法");
        let saturday = Local
            .with_ymd_and_hms(1999, 3, 13, 5, 0, 0)
            .single()
            .expect("固定時間合法");
        let listed = StockExchangeMarket::Listed;

        let counts = with_origins(&[(ISIN, &uri)], async {
            (
                visit_on(weekday, listed).await.expect("visit_on").len(),
                visit_on(saturday, listed).await.expect("visit_on").len(),
                all_listed_symbols_on(weekday, listed)
                    .await
                    .expect("all_listed")
                    .len(),
                all_listed_symbols_on(saturday, listed)
                    .await
                    .expect("all_listed")
                    .len(),
            )
        })
        .await;

        assert_eq!(counts, (4, 0, 4, 0));
    }

    /// 來源回 500 時不會拋錯，錯誤頁解析成空名冊；由呼叫端的名冊大小安全閥擋下，不會拿去做下市比對。
    #[tokio::test]
    async fn server_error_page_parses_to_empty_roster() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500).set_body_string("<html>error</html>"))
            .mount(&server)
            .await;
        let uri = server.uri();

        let html = with_origins(
            &[(ISIN, &uri)],
            fetch_page(StockExchangeMarket::OverTheCounter),
        )
        .await
        .expect("非 2xx 也取回內容");

        assert!(parse_all_symbols_html(&html).is_empty());
    }
}
