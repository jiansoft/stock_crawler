-- 2026-10-02 dividend 主鍵加入所屬年度，並保留「年度層級列每個發放年度一列」的唯一性。
--
-- 背景：舊主鍵 (security_code, year, quarter) 存不下同一發放年度的兩次同期別配息
-- （例如 3008 大立光 2022 年 1 月除息的 2021H1 與 8 月除息的 2022H1），後寫入的會覆蓋先寫入的。
--
-- 分三步執行，讓新舊版程式在部署期間都能寫入：
--   1. 先建兩個新的唯一索引（現有資料在舊主鍵下唯一，必然也滿足新索引），新版程式的
--      ON CONFLICT 目標因此可用，舊主鍵仍在，舊版程式也照常運作。
--   2. 部署新版程式。
--   3. 以新索引取代舊主鍵。

-- 步驟 1（部署前）
create unique index if not exists dividend_identity_uidx
    on public.dividend (security_code, year, year_of_dividend, quarter);
create unique index if not exists dividend_annual_level_uidx
    on public.dividend (security_code, year) where quarter = '';

-- 步驟 3（部署後）
-- alter table public.dividend
--     drop constraint dividend_pkey,
--     add constraint dividend_pkey primary key using index dividend_identity_uidx;
