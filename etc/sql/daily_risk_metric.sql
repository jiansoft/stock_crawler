-- 每日風險指標（低波動選股，2026-11-09）。
--
-- 由 stock_crawler 收盤寫入相對強度之後計算：上市櫃（含 ETF）每檔以截至當天的近 240 筆日線漲跌幅（參考價為
-- 基準，除權息不算下跌）算年化波動度與對加權指數日報酬的 beta；樣本不足 200 筆不寫入。同一天重算時整天替換；
-- 條件選股與回測的 max_volatility_percent、max_beta 與低波動排序讀這張表。
CREATE TABLE IF NOT EXISTS daily_risk_metric
(
    date               DATE                                   NOT NULL, -- 交易日
    stock_symbol       VARCHAR(24)                            NOT NULL, -- 股票代號
    volatility_percent NUMERIC(8, 2)                          NOT NULL, -- 年化波動度（%）
    beta               NUMERIC(8, 3),                                   -- 對加權指數的 beta
    samples            SMALLINT                               NOT NULL, -- 樣本筆數（200–240）
    created_at         TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    PRIMARY KEY (date, stock_symbol)
);

-- 依股票查最新一筆（條件選股）。
CREATE INDEX IF NOT EXISTS "daily_risk_metric-stock_symbol-date-idx"
    ON daily_risk_metric (stock_symbol, date DESC);

COMMENT ON TABLE daily_risk_metric IS '每日風險指標（近 240 筆日線的年化波動度與對加權指數的 beta；收盤寫入相對強度後計算）';
COMMENT ON COLUMN daily_risk_metric.date IS '交易日';
COMMENT ON COLUMN daily_risk_metric.stock_symbol IS '股票代號';
COMMENT ON COLUMN daily_risk_metric.volatility_percent IS '年化波動度（%）：近 240 筆日漲跌幅的樣本標準差 × √240';
COMMENT ON COLUMN daily_risk_metric.beta IS '對加權指數的 beta：同一段日漲跌幅對加權指數日報酬的迴歸斜率；同日沒有指數時不計入';
COMMENT ON COLUMN daily_risk_metric.samples IS '樣本筆數（日線列數，200–240）';
COMMENT ON COLUMN daily_risk_metric.created_at IS '寫入時間';
