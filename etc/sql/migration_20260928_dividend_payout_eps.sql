-- 盈餘分配率改為「股利 ÷ 這筆股利涵蓋期間的 EPS」（見 src/domain/dividend/payout.rs），
-- 分母 EPS 與涵蓋期間一併寫回，股利分配表顯示的 EPS 才會與分配率出自同一組數字。
-- 新表由 dividend.sql 建立時已含這兩欄；這支 migration 只用於既有資料庫，可重複執行。
ALTER TABLE public.dividend ADD COLUMN IF NOT EXISTS payout_eps numeric(18, 4);
ALTER TABLE public.dividend ADD COLUMN IF NOT EXISTS payout_period varchar(16);

COMMENT ON COLUMN public.dividend.payout_eps IS '盈餘分配率的分母：這筆股利涵蓋期間的每股盈餘（尚未計算時為 NULL）';
COMMENT ON COLUMN public.dividend.payout_period IS '股利涵蓋的盈餘期間，例如 2025、2026Q2、2025Q4~2026Q2（尚未計算時為 NULL）';
