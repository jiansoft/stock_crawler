/// 從 yahoo 取回股票的股利數據
pub mod dividend;
/// 從 yahoo 取回股票的基本數據
pub mod profile;
/// 即時報價
pub mod price;

const HOST: &str = "tw.stock.yahoo.com";
