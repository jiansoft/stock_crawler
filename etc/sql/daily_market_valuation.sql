-- 每日市場估值水位（2026-10-20）。
--
-- 由 stock_crawler 在官方本益比補值（17:30、20:30）之後重算最近 7 天：當天上市、上櫃與兩者合計
-- （stock_exchange_market_id = 0）股票的官方本益比與股價淨值比中位數，只計大於 0 的值（虧損股沒有本益比、
-- ETF 兩者都沒有）。/market/valuation、條件選股回測的大盤估值濾網與每週市場摘要讀這張表。
CREATE TABLE IF NOT EXISTS daily_market_valuation
(
    date                     DATE                                   NOT NULL, -- 交易日
    stock_exchange_market_id INTEGER                                NOT NULL, -- 0 上市櫃合計、2 上市、4 上櫃
    median_pe                NUMERIC(12, 4),                                  -- 本益比中位數；當天沒有時為 NULL
    median_pb                NUMERIC(12, 4),                                  -- 股價淨值比中位數；當天沒有時為 NULL
    pe_samples               INTEGER                                NOT NULL, -- 計入本益比中位數的股票數
    pb_samples               INTEGER                                NOT NULL, -- 計入股價淨值比中位數的股票數
    updated_at               TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
    CONSTRAINT daily_market_valuation_pkey PRIMARY KEY (date, stock_exchange_market_id)
);

COMMENT ON TABLE daily_market_valuation IS '每日市場估值水位：上市、上櫃與合計的官方本益比、股價淨值比中位數（只計大於 0）';
COMMENT ON COLUMN daily_market_valuation.stock_exchange_market_id IS '0 上市櫃合計、2 上市、4 上櫃';
COMMENT ON COLUMN daily_market_valuation.median_pe IS '本益比中位數（只計大於 0）；當天沒有時為 NULL';
COMMENT ON COLUMN daily_market_valuation.median_pb IS '股價淨值比中位數（只計大於 0）；當天沒有時為 NULL';
COMMENT ON COLUMN daily_market_valuation.pe_samples IS '計入本益比中位數的股票數';
COMMENT ON COLUMN daily_market_valuation.pb_samples IS '計入股價淨值比中位數的股票數';
