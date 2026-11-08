//! ETF 相對代理基準的追蹤誤差；明確區分追蹤指數名稱及實際比較基準的測試。

use super::*;
#[tokio::test]
async fn rejects_invalid_symbol_before_database_access() {
    for symbol in ["", "0050;DROP", "xxxxxxxxxxxxx", "台積電"] {
        assert_eq!(
            etf_tracking(Path(symbol.into())).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}
