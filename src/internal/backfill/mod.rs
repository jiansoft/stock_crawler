/// 調用 twse API 更新終止上市公司
pub mod delisted_company;
/// 回補財報
pub mod financial_statement;
/// 調用 twse API 取得數據後更新股票相關欄位
pub mod international_securities_identification_number;
/// 回補每股淨值為零的股票更新其數據
pub mod net_asset_value_per_share;
/// 調用 twse API 取得每月營收
pub mod revenue;
/// 調用 twse API 取得台股加權指數
pub mod taiwan_capitalization_weighted_stock_index;
/// 調用 twse、tpex API 取得台股收盤報價
pub mod quote;