// 持股股利現金流預估（已公告與依去年同期推估）。
pub mod dividend_forecast;
pub mod entity;
// 持股健檢的基本面警示（營收連續年減、三率三降、營業虧損）。
pub mod health;
// 持股績效（時間加權報酬、月報酬、最大回撤、與大盤比較）。
pub mod performance;
pub mod repository;
/// 持股組合總覽（市值、權重、損益、產業分布與加權指標）。
pub mod summary;
