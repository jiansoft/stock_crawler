/// 調用 twse API 更新終止上市公司
pub mod delisted_company;
/// 更新股利發送數據
pub mod dividend;
/// 回補財報
pub mod financial_statement;
/// 調用 twse API 取得數據後更新股票相關欄位
pub mod isin;
/// 回補每股淨值為零的股票更新其數據
pub mod net_asset_value_per_share;
/// 調用 twse、tpex API 取得並更新台股收盤報價
pub mod quote;
/// 調用 twse API 取得並更新每月營收
pub mod revenue;
/// 查詢 taifex 提供個股權值比重
pub mod stock_weight;
/// 調用 twse API 取得並更新台股加權指數
pub mod taiwan_stock_index;
/// 外資及陸資投資持股統計
pub mod qualified_foreign_institutional_investor;
