//! net_asset_value_per_share的測試；連外的測試標 #[ignore]。

use super::*;

/// 最小 HTML fixture：標頭列放在 `<thead>` 讓選擇器自動跳過，
/// tbody 中含 2 筆有效資料列與 1 筆欄數不足的短列。
const FIXTURE_HTML: &str = r#"<html><body>
<table id="company_list">
<thead>
  <tr><th>序號</th><th>代號</th><th>名稱</th><th>股數</th><th>盈虧</th><th>每股淨值</th></tr>
</thead>
<tbody>
  <tr><td>1</td><td>5971</td><td>某公司</td><td>10000000</td><td>1000</td><td>12.34</td></tr>
  <tr><td>2</td><td>6001</td><td>另一公司</td><td>5000000</td><td>-500</td><td>8.56</td></tr>
  <tr><td>只有四欄</td><td>X</td><td>Y</td><td>Z</td></tr>
</tbody>
</table>
</body></html>"#;

#[test]
fn test_parse_emerging_html_basic() {
    let result = parse_emerging_html(FIXTURE_HTML);
    // 標頭列 + 欄數不足列都應被略過，只保留 2 筆有效資料
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].stock_symbol, "5971");
    assert_eq!(
        result[0].net_asset_value_per_share,
        Decimal::from_str_exact("12.34").unwrap()
    );
    assert_eq!(result[1].stock_symbol, "6001");
    assert_eq!(
        result[1].net_asset_value_per_share,
        Decimal::from_str_exact("8.56").unwrap()
    );
}

#[test]
fn test_parse_emerging_html_skips_short_rows() {
    let html = r#"<html><body><table id="company_list"><tbody>
<tr><td>A</td><td>B</td><td>C</td></tr>
</tbody></table></body></html>"#;
    let result = parse_emerging_html(html);
    assert!(result.is_empty());
}

#[test]
fn test_parse_emerging_html_invalid_decimal_defaults_to_zero() {
    let html = r#"<html><body><table id="company_list"><tbody>
<tr><td>1</td><td>9999</td><td>測試</td><td>0</td><td>0</td><td>N/A</td></tr>
</tbody></table></body></html>"#;
    let result = parse_emerging_html(html);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].net_asset_value_per_share, Decimal::ZERO);
}

#[tokio::test]
#[ignore]
async fn test_visit() {
    dotenvy::dotenv().ok();
    tracing::debug!("開始 visit");

    match visit().await {
        Err(why) => {
            tracing::error!("Failed to visit because {:?}", why);
        }
        Ok(list) => {
            tracing::debug!("data({}):{:#?}", list.len(), list);
        }
    }

    tracing::debug!("結束 visit");
}
