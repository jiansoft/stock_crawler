//! HiStock 排行頁與單列解析的測試。

use super::*;

#[test]
fn test_parse_full_row_from_user_sample() {
    let html = r#"<table><tr class="alt-row">
			<td>5274</td><td>信驊</td><td><span class="price-down">9445</span></td>
        <td><span class="price-down">▼-55.00</span></td><td><span class="price-down">-0.58%</span></td>
        <td>-3.18%</td><td>2.95%</td><td>9620</td><td>9715</td><td>9435</td><td>9445</td><td>44</td><td>4.156</td>
		</tr></table>"#;
    let fragment = Html::parse_fragment(html);
    let tr_selector = Selector::parse("tr").expect("Failed to parse tr selector");
    let row = fragment.select(&tr_selector).next().unwrap();
    let (symbol, snapshot) = parse_row(row).unwrap().unwrap();

    assert_eq!(symbol, "5274");
    assert_eq!(snapshot.name, "信驊");
    assert_eq!(snapshot.source_site, "HiStock");
    assert_eq!(snapshot.price, dec!(9445));
    assert_eq!(snapshot.change, dec!(-55));
}

#[test]
fn test_parse_no_change_row() {
    let html = r#"<table><tr><td>6584</td><td>南俊國際</td><td>425</td><td>--</td><td>--</td><td>...</td><td>...</td><td>423.5</td><td>435</td><td>423.5</td><td>425</td><td>148</td><td>0.629</td></tr></table>"#;
    let fragment = Html::parse_fragment(html);
    let tr_selector = Selector::parse("tr").expect("Failed to parse tr selector");
    let row = fragment.select(&tr_selector).next().unwrap();
    let (symbol, snapshot) = parse_row(row).unwrap().unwrap();

    assert_eq!(symbol, "6584");
    assert_eq!(snapshot.source_site, "HiStock");
    assert_eq!(snapshot.change, Decimal::ZERO);
    assert_eq!(snapshot.volume, dec!(148));
}

#[test]
fn parse_row_skips_header_and_non_numeric_symbols() {
    let html = r#"<table>
        <tr><td>代號</td><td>名稱</td></tr>
        <tr><td>ABC</td><td>非股票列</td></tr>
    </table>"#;
    let fragment = Html::parse_fragment(html);
    let tr_selector = Selector::parse("tr").expect("Failed to parse tr selector");

    for row in fragment.select(&tr_selector) {
        assert!(parse_row(row).unwrap().is_none());
    }
}

#[test]
fn parse_row_keeps_positive_change_and_range() {
    let html = r#"<table><tr>
        <td>2330</td><td>台積電</td><td>1000</td>
        <td><span class="price-up">▲+15.00</span></td>
        <td><span class="price-up">+1.52%</span></td>
        <td>...</td><td>...</td><td>990</td><td>1005</td><td>985</td><td>985</td><td>1,234</td>
    </tr></table>"#;
    let fragment = Html::parse_fragment(html);
    let tr_selector = Selector::parse("tr").expect("Failed to parse tr selector");
    let row = fragment.select(&tr_selector).next().unwrap();
    let (symbol, snapshot) = parse_row(row).unwrap().unwrap();

    assert_eq!(symbol, "2330");
    assert_eq!(snapshot.change, dec!(15));
    assert_eq!(snapshot.change_range, dec!(1.52));
    assert_eq!(snapshot.open, dec!(990));
    assert_eq!(snapshot.high, dec!(1005));
    assert_eq!(snapshot.low, dec!(985));
    assert_eq!(snapshot.last_close, dec!(985));
    assert_eq!(snapshot.volume, dec!(1234));
}

#[test]
fn parse_row_reports_malformed_numeric_fields() {
    let html = r#"<table><tr>
        <td>2330</td><td>台積電</td><td>not-a-price</td>
        <td>--</td><td>--</td><td>...</td><td>...</td>
        <td>990</td><td>1005</td><td>985</td><td>985</td><td>1234</td>
    </tr></table>"#;
    let fragment = Html::parse_fragment(html);
    let tr_selector = Selector::parse("tr").expect("Failed to parse tr selector");
    let row = fragment.select(&tr_selector).next().unwrap();
    let error = parse_row(row).unwrap_err().to_string();

    assert!(error.contains("Failed to parse price for 2330"));
}

/// 以貼近真實排行榜頁面形狀的 fixture 驗證整頁解析流程。
///
/// 上面的 `test_parse_*` 系列只餵單一 `<tr>` 給 `parse_row`，
/// 這裡則走 [`parse_rank_html`] 完整路徑，額外驗證：
/// `#CPHB1_gv tr` 選擇器命中、表頭列略過、含英文字母代號
/// （債券 ETF）略過，以及 row_count / body_bytes 診斷數字正確。
#[test]
fn parse_rank_html_parses_fixture_rows() {
    // include_str! 的路徑相對於本檔案（histock/price/tests.rs）→ histock/testdata/。
    // 在「編譯期」把檔案內容嵌進測試執行檔，執行時不做任何檔案 I/O。
    const FIXTURE: &str = include_str!("../../testdata/rank_page.html");

    let result = parse_rank_html(FIXTURE).unwrap();

    // fixture 有 5 個 <tr>：表頭 + 4 檔股票；全部都要被掃到（診斷用）。
    assert_eq!(result.row_count, 5);
    assert_eq!(result.body_bytes, FIXTURE.len());
    // 有效快照只有 3 檔：表頭列（代號非數字）與 00687B（含英文字母的
    // 債券 ETF 代號，依現行規則不納入）都被略過。
    assert_eq!(result.snapshots.len(), 3);
    assert!(!result.snapshots.contains_key("00687B"));

    // 下跌列：▼ 前綴 → 漲跌與幅都轉為負值。
    let down = &result.snapshots["5274"];
    assert_eq!(down.name, "信驊");
    assert_eq!(down.price, dec!(9445));
    assert_eq!(down.change, dec!(-55));
    assert_eq!(down.change_range, dec!(-0.58));

    // 上漲列：▲ 與 + 前綴維持正值；成交量的千分位逗號要被正確去除。
    let up = &result.snapshots["2330"];
    assert_eq!(up.change, dec!(15));
    assert_eq!(up.change_range, dec!(1.52));
    assert_eq!(up.volume, dec!(31415));
    assert_eq!(up.last_close, dec!(985));

    // 平盤列：「--」的漲跌與幅視為 0，不是解析錯誤。
    let flat = &result.snapshots["6584"];
    assert_eq!(flat.change, Decimal::ZERO);
    assert_eq!(flat.change_range, Decimal::ZERO);
}

/// 整頁完全沒有可用列（例如 HiStock 改版換了表格 id）時必須報錯，
/// 讓上層知道快取「這一輪沒有更新」，而不是默默拿空資料覆蓋。
#[test]
fn parse_rank_html_rejects_page_without_rank_table() {
    let error = parse_rank_html("<html><body><p>維護中</p></body></html>").unwrap_err();
    assert!(error.to_string().contains("empty map"));
}
