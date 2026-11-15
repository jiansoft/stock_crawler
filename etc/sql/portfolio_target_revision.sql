-- 投資組合目標配置的歷史版本（2026-10-28）。
--
-- portfolio_target 的儲存是「整個維度先刪再寫」，改錯就回不去；每次儲存前，若該維度原本已設定、
-- 且內容有變，就把儲存前的整組配置記成一版。每個會員每個維度只保留最近 20 版（由程式清除）。
CREATE TABLE IF NOT EXISTS portfolio_target_revision
(
    id           BIGSERIAL PRIMARY KEY,
    household_id BIGINT                   NOT NULL DEFAULT 1,
    member_id    VARCHAR(64)              NOT NULL,
    dimension  VARCHAR(32)              NOT NULL CHECK (dimension IN ('stock', 'industry', 'etf_look_through')),
    targets    JSONB                    NOT NULL,
    saved_time TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    actor_source VARCHAR(8)   NOT NULL DEFAULT '', -- web / mcp / 空字串 = 舊資料
    actor_ref    VARCHAR(64)  NOT NULL DEFAULT ''  -- web: 操作會員編號；mcp: Token id
);

COMMENT ON TABLE portfolio_target_revision IS '投資組合目標配置的歷史版本：每次儲存前的整組配置';
COMMENT ON COLUMN portfolio_target_revision.member_id IS '使用者帳號或會員ID';
COMMENT ON COLUMN portfolio_target_revision.dimension IS '配置維度：stock（個股）、industry（產業）、etf_look_through（ETF穿透）';
COMMENT ON COLUMN portfolio_target_revision.targets IS '這次儲存之前的整組配置：[{"key": 代號或產業, "target_percent": 百分比}]，空陣列為全現金';
COMMENT ON COLUMN portfolio_target_revision.saved_time IS '被取代（記成這一版）的時間';
COMMENT ON COLUMN portfolio_target_revision.household_id IS '所屬家庭；Phase 7 前只有家庭 #1';
COMMENT ON COLUMN portfolio_target_revision.actor_source IS '取代這一組的那次寫入來源：web、mcp；空字串為舊資料';
COMMENT ON COLUMN portfolio_target_revision.actor_ref IS '取代這一組的那次寫入參照：web 為操作會員編號、mcp 為 Token id';

CREATE INDEX IF NOT EXISTS "portfolio_target_revision-member-dimension-time-idx"
    ON portfolio_target_revision (member_id, dimension, saved_time DESC);
CREATE INDEX IF NOT EXISTS idx_portfolio_target_revision_household
    ON portfolio_target_revision (household_id, member_id, dimension, saved_time DESC);
