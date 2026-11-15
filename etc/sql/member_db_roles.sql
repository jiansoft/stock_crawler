-- =============================================================================
-- 檔案：etc/sql/member_db_roles.sql（會員系統計畫 4.7；上線程序 L0）
--
-- 建立各服務專用的資料庫角色、把既有物件的擁有者移轉給 stock_owner、授予既有業務表的權限。
-- IAM 表（"Member"、member_*、notify_topic）的權限不在這裡：L0 時它們大多還不存在，
-- 一律由 member_db_grants_iam.sql（L1，結構檔之後）逐表授予。"Member" 在 L0 已存在，
-- 這裡只給 stock_go_app 維持現狀所需的權限（dividend_record_repository 以 "Member" 取姓名）。
--
-- 執行者：超級使用者，連到目標資料庫執行（psql -d <資料庫> -f）。可重複執行。
-- 不含密碼：各登入角色的密碼由管理者另行以 ALTER ROLE … PASSWORD 設定，不進版控。
-- 不得出現：ALTER ROLE pi、DROP ROLE、指名資料庫的 ALTER DATABASE、REASSIGN OWNED
-- （角色是整個叢集共用的；REASSIGN OWNED 會跨資料庫改到共用物件）。擁有者移轉一律逐物件 ALTER … OWNER TO。
-- =============================================================================

-- =============================================================================
-- 1. 角色（叢集層級；已存在就略過）
-- =============================================================================
DO $$
DECLARE
    r record;
BEGIN
    FOR r IN
        SELECT * FROM (VALUES
            ('stock_owner',    false),  -- 擁有全部物件；只在人工執行 DDL／遷移時使用
            ('stock_rust_app', true),   -- stock_rust
            ('stock_go_app',   true),   -- stock_go 的 API 與一般排程
            ('stock_go_maint', true),   -- stock_go 的每日清理與資料權利工具（獨立連線池）
            ('stock_backup',   true),   -- 只供 pg_dump
            ('stock_readonly', true)    -- 其他唯讀用途（stock_api、手動查詢）
        ) AS t(name, can_login)
    LOOP
        IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = r.name) THEN
            EXECUTE format('CREATE ROLE %I %s', r.name, CASE WHEN r.can_login THEN 'LOGIN' ELSE 'NOLOGIN' END);
        END IF;
    END LOOP;
END $$;

-- 內建角色：stock_rust 的資料庫健康檢查讀 pg_stat_statements／pg_stat_activity，
-- 沒有 pg_read_all_stats 時只看得到自己的查詢（不報錯、數字靜默變少）
GRANT pg_read_all_stats TO stock_rust_app;
-- 備份角色可讀全部表（含密碼雜湊與加密後的 MFA 密鑰），所以備份檔要加密保存
GRANT pg_read_all_data TO stock_backup;

-- 既有帳號 pi：擁有者移轉後仍保有與移轉前相同的權限，服務把連線帳號改回 pi 即可退版。
-- L3 切到 enforce 時另行 REVOKE stock_owner FROM pi 並停用登入（不在本檔）。
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pi') THEN
        GRANT stock_owner TO pi;
    END IF;
END $$;

-- =============================================================================
-- 2. 連線與 schema 使用權（資料庫名稱取當下連線，不寫死）
-- =============================================================================
DO $$
BEGIN
    EXECUTE format('GRANT CONNECT ON DATABASE %I TO stock_rust_app, stock_go_app, stock_go_maint, stock_backup, stock_readonly',
                   current_database());
END $$;
GRANT USAGE ON SCHEMA public TO stock_owner, stock_rust_app, stock_go_app, stock_go_maint, stock_readonly;
GRANT CREATE ON SCHEMA public TO stock_owner;

-- =============================================================================
-- 3. 擁有者移轉：public 內不屬於擴充套件的表、視圖、序列、函式與型別
-- =============================================================================
DO $$
DECLARE
    r record;
BEGIN
    -- 表、分割表、視圖、具體化視圖、外部表（索引與欄位綁定的序列會跟著表一起移轉）
    FOR r IN
        SELECT c.oid::regclass AS obj, c.relkind
        FROM pg_class AS c
        WHERE c.relnamespace = 'public'::regnamespace
          AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
          AND c.relowner <> 'stock_owner'::regrole
          AND NOT EXISTS (SELECT 1 FROM pg_depend AS d
                          WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'e')
    LOOP
        EXECUTE format('ALTER %s %s OWNER TO stock_owner',
                       CASE r.relkind WHEN 'v' THEN 'VIEW'
                                      WHEN 'm' THEN 'MATERIALIZED VIEW'
                                      WHEN 'f' THEN 'FOREIGN TABLE'
                                      ELSE 'TABLE' END,
                       r.obj);
    END LOOP;

    -- 獨立序列（綁定欄位的 serial／identity 序列已隨表移轉，不能單獨改）
    FOR r IN
        SELECT c.oid::regclass AS obj
        FROM pg_class AS c
        WHERE c.relnamespace = 'public'::regnamespace
          AND c.relkind = 'S'
          AND c.relowner <> 'stock_owner'::regrole
          AND NOT EXISTS (SELECT 1 FROM pg_depend AS d
                          WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid
                            AND d.deptype IN ('a', 'i', 'e'))
    LOOP
        EXECUTE format('ALTER SEQUENCE %s OWNER TO stock_owner', r.obj);
    END LOOP;

    -- 函式、程序與彙總函式
    FOR r IN
        SELECT p.oid::regprocedure AS obj, p.prokind
        FROM pg_proc AS p
        WHERE p.pronamespace = 'public'::regnamespace
          AND p.proowner <> 'stock_owner'::regrole
          AND NOT EXISTS (SELECT 1 FROM pg_depend AS d
                          WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e')
    LOOP
        EXECUTE format('ALTER %s %s OWNER TO stock_owner',
                       CASE r.prokind WHEN 'a' THEN 'AGGREGATE' ELSE 'ROUTINE' END, r.obj);
    END LOOP;

    -- 列舉、範圍、網域與獨立複合型別（表的列型別與陣列型別會跟著原物件）
    FOR r IN
        SELECT t.oid::regtype AS obj, t.typtype
        FROM pg_type AS t
        LEFT JOIN pg_class AS c ON c.oid = t.typrelid
        WHERE t.typnamespace = 'public'::regnamespace
          AND t.typowner <> 'stock_owner'::regrole
          AND t.typtype IN ('e', 'r', 'd', 'c')
          AND (t.typtype <> 'c' OR c.relkind = 'c')
          AND NOT EXISTS (SELECT 1 FROM pg_depend AS d
                          WHERE d.classid = 'pg_type'::regclass AND d.objid = t.oid AND d.deptype = 'e')
    LOOP
        EXECUTE format('ALTER %s %s OWNER TO stock_owner',
                       CASE r.typtype WHEN 'd' THEN 'DOMAIN' ELSE 'TYPE' END, r.obj);
    END LOOP;
END $$;

-- =============================================================================
-- 4. 既有業務表的權限（IAM 表除外；維持兩個服務現有的讀寫能力）
-- =============================================================================
-- IAM 表的判定與 member_db_grants_iam.sql 相同："Member"、member_ 開頭、notify_topic。
-- 個人業務表（計畫 4.0 第 ⑦ 區，13 張）兩個服務照常讀寫，但 stock_readonly 不可讀。
DO $$
DECLARE
    r record;
    personal_tables CONSTANT text[] := ARRAY[
        'stock_ownership_details',
        'dividend_record_detail', 'dividend_record_detail_more',
        'sell_history',
        'daily_money_history_member', 'daily_money_history_detail', 'daily_money_history_detail_more',
        'daily_money_history',
        'dividend_forecast_snapshot',
        'portfolio_target', 'portfolio_target_dimension', 'portfolio_target_revision',
        'trace'
    ];
BEGIN
    FOR r IN
        SELECT c.oid::regclass AS obj, c.relname, c.relkind
        FROM pg_class AS c
        WHERE c.relnamespace = 'public'::regnamespace
          AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
          AND NOT (c.relname = 'Member' OR c.relname LIKE 'member\_%' OR c.relname = 'notify_topic')
          AND NOT EXISTS (SELECT 1 FROM pg_depend AS d
                          WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'e')
    LOOP
        IF r.relkind IN ('v', 'm') THEN
            EXECUTE format('GRANT SELECT ON %s TO stock_rust_app, stock_go_app', r.obj);
        ELSE
            EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON %s TO stock_rust_app, stock_go_app', r.obj);
        END IF;
        IF NOT r.relname = ANY (personal_tables) THEN
            EXECUTE format('GRANT SELECT ON %s TO stock_readonly', r.obj);
        END IF;
    END LOOP;

    -- 有流水號的表新增資料時要用到序列（"Member" 的序列由 IAM 授權檔處理）
    FOR r IN
        SELECT c.oid::regclass AS obj
        FROM pg_class AS c
        WHERE c.relnamespace = 'public'::regnamespace
          AND c.relkind = 'S'
          AND c.relname NOT LIKE 'Member\_%'
          AND c.relname NOT LIKE 'member\_%'
          AND c.relname NOT LIKE 'notify\_topic\_%'
          AND NOT EXISTS (SELECT 1 FROM pg_depend AS d
                          WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'e')
    LOOP
        EXECUTE format('GRANT USAGE, SELECT ON SEQUENCE %s TO stock_rust_app, stock_go_app', r.obj);
    END LOOP;

    -- 兩個服務在執行期整表重建 last_daily_quotes（stock_rust last_daily_quotes.rs、stock_go dailyquote.go）
    IF to_regclass('public.last_daily_quotes') IS NOT NULL THEN
        GRANT TRUNCATE ON public.last_daily_quotes TO stock_rust_app, stock_go_app;
    END IF;

    -- "Member" 在 L0 已存在：stock_go 讀取會員姓名、建立會員（維持現狀）；stock_rust 目前不讀這張表
    IF to_regclass('public."Member"') IS NOT NULL THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON public."Member" TO stock_go_app;
        GRANT USAGE, SELECT ON SEQUENCE public."Member_Id_seq" TO stock_go_app;
    END IF;
END $$;

-- =============================================================================
-- 5. 預設權限：日後由 stock_owner 新建的一般業務表與序列自動授權給兩個服務
-- =============================================================================
-- IAM 表不走預設授權：member_db_grants_iam.sql 會先撤銷 IAM 表上的全部權限，再逐表列出。
-- 新增個人業務表時，同步更新上面的 personal_tables 與計畫 4.0 第 ⑦ 區清單（stock_readonly 不得讀取）。
ALTER DEFAULT PRIVILEGES FOR ROLE stock_owner IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO stock_rust_app, stock_go_app;
ALTER DEFAULT PRIVILEGES FOR ROLE stock_owner IN SCHEMA public
    GRANT USAGE, SELECT ON SEQUENCES TO stock_rust_app, stock_go_app;
