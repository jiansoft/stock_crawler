-- =============================================================================
-- 檔案：etc/sql/member_rbac_finalize.sql（會員系統計畫 4.2、上線程序 L3）
-- 會員系統的收尾檔：移除結構檔為「退回舊版程式」保留的過渡索引與預設值。
--
-- 執行時機：正式庫在上線程序 L3 確認通過後執行（此後不再退回合併前的舊版程式），
-- 而且必須在建立第二個家庭之前：過渡唯一索引只看 (date, member_id)，
-- 兩個家庭同一天的合計列（member_id = 0）會互相衝突，讓每日結算失敗。
-- CI 與測試庫在結構檔之後直接載入，測試的是收尾後的最終結構。
-- 可重複執行。執行後若又重跑結構檔，過渡索引與預設值會被加回，須再跑一次本檔。
-- =============================================================================

BEGIN;

-- 依家庭結算後，舊欄位組合的唯一索引會誤擋不同家庭的合計。
DROP INDEX IF EXISTS public.idx_daily_money_history_member_legacy_key;
DROP INDEX IF EXISTS public.idx_daily_money_history_detail_legacy_key;
-- 配置目標維度：兩個家庭各有全家目標（member_id = '0'）時，舊欄位組合會互相衝突。
DROP INDEX IF EXISTS public.idx_portfolio_target_dimension_legacy_key;

-- 新增資料不可依賴隱含的家庭或會員：省略時由 NOT NULL 拒絕。
ALTER TABLE public.daily_money_history_member ALTER COLUMN household_id DROP DEFAULT;
ALTER TABLE public.daily_money_history_detail ALTER COLUMN household_id DROP DEFAULT;
ALTER TABLE public.daily_money_history_detail_more ALTER COLUMN household_id DROP DEFAULT;
ALTER TABLE public.daily_money_history_member ALTER COLUMN settlement_household_id DROP DEFAULT;
ALTER TABLE public.daily_money_history_detail ALTER COLUMN settlement_household_id DROP DEFAULT;
ALTER TABLE public.daily_money_history_detail_more ALTER COLUMN settlement_household_id DROP DEFAULT;
ALTER TABLE public.trace ALTER COLUMN member_id DROP DEFAULT;

-- trace 的過渡唯一索引在 L2 已移除；這裡再確認一次，避免遺漏。
DROP INDEX IF EXISTS public.idx_trace_legacy_symbol;

COMMIT;
