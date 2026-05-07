/// 調用 twse API 更新終止上市公司
pub mod delisted_company;
/// 更新股利發送數據
pub mod dividend;
/// 調用 twse API 取得 ETF 並更新股票相關欄位
pub mod etf;
/// 回補財報
pub mod financial_statement;
/// 調用 twse API 取得數據後更新股票相關欄位
pub mod isin;
/// 回補每股淨值為零的股票更新其數據
pub mod net_asset_value_per_share;
/// 外資及陸資投資持股統計
pub mod qualified_foreign_institutional_investor;
/// 調用 twse、tpex API 取得並更新台股收盤報價
pub mod quote;
/// 調用 twse API 取得並更新每月營收
pub mod revenue;
/// 查詢 taifex 提供個股權值比重
pub mod stock_weight;
/// 調用 twse API 取得並更新台股加權指數
pub mod taiwan_stock_index;

use crate::cache::SHARE;

/// 判斷股票主檔是否為新資料，或關鍵欄位是否有變動。
pub(crate) async fn is_stock_identity_new_or_changed(
    stock_symbol: &str,
    industry_id: i32,
    stock_exchange_market_id: i32,
    name: &str,
) -> bool {
    match SHARE.get_stock(stock_symbol).await {
        // 情況 A：資料庫已存在該股票 (Some)
        Some(stock_db)
            // 檢查關鍵欄位是否有變動：產業 ID、市場 ID 或名稱
            if stock_db.stock_industry_id != industry_id
                || stock_db.stock_exchange_market_id != stock_exchange_market_id
                || stock_db.name != name =>
        {
            // 有任一欄位不同，標記為需要更新
            true
        }
        // 情況 B：資料庫完全找不到這檔股票 (None)
        None => true,
        // 情況 C：資料完全一致
        _ => false,
    }
}
