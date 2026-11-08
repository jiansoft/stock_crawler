// 固定投入 10,000 元的各期間年化報酬率（CAGR）
pub mod cagr;
/// 股票每日行情
pub mod daily_quotes;
/// 每日重算除權息填息事件（條件選股與回測的股利條件用）
pub mod dividend_fill_event;
/// 計算股票股息收入
pub mod dividend_fill_hints;
pub mod dividend_record;
/// 估算便宜、合理、昂貴價
pub mod estimated_price;
pub mod etf_performance;
/// 每日重算全市場財務體質（F 分數與選股季報指標）
pub mod financial_health;
// 收盤後回填訊號績效用的未來報酬
pub mod forward_return;
/// 計算每日市值
pub mod money_history;
/// 收盤後寫入每日相對強度（`daily_relative_strength`）。
pub mod relative_strength;
/// 收盤後全市場技術訊號
pub mod technical_signal;
/// 持股未來已公告的除權息與預估入帳
pub mod upcoming_dividends;
