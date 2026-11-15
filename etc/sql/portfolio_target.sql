-- 投資組合目標配置（2026-10-25）。
--
-- 記錄使用者在各維度（個股 stock、產業 industry、ETF 穿透 etf_look_through）設定的目標權重百分比。
-- 同一使用者同維度的權重合計必須 <= 100，剩餘視為現金或未指定。
-- 會員系統：依家庭分開，member_id = '0' 是該家庭的全家目標（household_id 預設 1，保留到 Phase 7）。
CREATE TABLE IF NOT EXISTS portfolio_target
(
    household_id   BIGINT                   DEFAULT 1                 NOT NULL, -- 所屬家庭
    member_id      VARCHAR(64)                                        NOT NULL, -- 使用者帳號或會員 ID
    dimension      VARCHAR(32)                                        NOT NULL, -- 配置維度：stock, industry, etf_look_through
    key            VARCHAR(64)                                        NOT NULL, -- 維度鍵值：股票代號或產業名稱
    target_percent NUMERIC(5, 2) CHECK (target_percent BETWEEN 0 AND 100) NOT NULL, -- 目標百分比（0.00 ~ 100.00）
    updated_time   TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    PRIMARY KEY (household_id, member_id, dimension, key)
);

COMMENT ON TABLE portfolio_target IS '投資組合目標配置比例（個股、產業、ETF穿透）';
COMMENT ON COLUMN portfolio_target.household_id IS '所屬家庭；Phase 7 前只有家庭 #1';
COMMENT ON COLUMN portfolio_target.member_id IS '使用者帳號或會員ID；0 為該家庭的全家目標';
COMMENT ON COLUMN portfolio_target.dimension IS '配置維度：stock（個股）、industry（產業）、etf_look_through（ETF穿透）';
COMMENT ON COLUMN portfolio_target.key IS '維度鍵值：股票代號或產業名稱';
COMMENT ON COLUMN portfolio_target.target_percent IS '目標配置百分比（0.00 至 100.00）';
COMMENT ON COLUMN portfolio_target.updated_time IS '最後更新時間';

-- 設定存在性獨立記錄，使空清單也能表示明確的 100% 現金目標。
CREATE TABLE IF NOT EXISTS portfolio_target_dimension (
    household_id BIGINT NOT NULL DEFAULT 1,
    member_id VARCHAR(64) NOT NULL,
    dimension VARCHAR(32) NOT NULL CHECK (dimension IN ('stock', 'industry', 'etf_look_through')),
    updated_time TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (household_id, member_id, dimension)
);
INSERT INTO portfolio_target_dimension(household_id, member_id, dimension)
SELECT DISTINCT household_id, member_id, dimension FROM portfolio_target ON CONFLICT DO NOTHING;
