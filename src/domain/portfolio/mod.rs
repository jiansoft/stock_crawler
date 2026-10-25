/// 共用交易費率。
pub mod costs;
// 持股股利現金流預估（已公告與依去年同期推估）。
pub mod dividend_forecast;
// 持股籌碼日報（當日持股的法人、信用交易與主力進出，與通知共用門檻）。
pub mod chip;
pub mod entity;
/// 持股穿透曝險（持有 ETF 依前十大持股拆到成分股，與直接持股合併）。
pub mod exposure;
// 持股健檢的基本面警示（營收連續年減、三率三降、營業虧損、自由現金流為負）。
pub mod health;
// 持股績效（時間加權報酬、月報酬、最大回撤、與大盤比較）。
pub mod performance;
/// 持股再平衡純計算。
pub mod rebalance;
pub mod repository;
/// 持股組合總覽（市值、權重、損益、產業分布與加權指標）。
pub mod summary;
/// 目標配置與驗證。
pub mod target;
