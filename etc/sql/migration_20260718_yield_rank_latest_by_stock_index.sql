-- Phase 3 screen_stocks 會對每支上市櫃股票取得最新殖利率。
-- P0-4 EXPLAIN 實測舊索引在指標排序時產生 523,399 次 index search，耗時約
-- 1.89 秒；以 security_code 為前導、date DESC 為次欄可直接取最新一筆。
CREATE INDEX IF NOT EXISTS "yield_rank-security_code-date-desc-idx"
    ON public.yield_rank (security_code, date DESC) INCLUDE (yield);
