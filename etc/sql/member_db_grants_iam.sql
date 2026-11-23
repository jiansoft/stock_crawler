-- =============================================================================
-- 檔案：etc/sql/member_db_grants_iam.sql（會員系統計畫 4.7；上線程序 L1，結構檔之後）
--
-- IAM 表、視圖與欄位層級權限。IAM 表不走預設授權：先撤銷各服務角色在 IAM 表上的全部權限
-- （含 ALTER DEFAULT PRIVILEGES 自動給的），再逐表列出。
--
-- 執行者：超級使用者，連到目標資料庫執行。須在 member_db_roles.sql 與
-- member_rbac_and_mcp_token.sql 之後。可重複執行。
-- =============================================================================

-- =============================================================================
-- 1. 擁有者：結構檔若不是以 stock_owner 身分執行，新建的 IAM 物件在這裡移轉
-- =============================================================================
DO $$
DECLARE
    r record;
BEGIN
    FOR r IN
        SELECT c.oid::regclass AS obj, c.relkind
        FROM pg_class AS c
        WHERE c.relnamespace = 'public'::regnamespace
          AND c.relkind IN ('r', 'p', 'v', 'm')
          AND c.relowner <> 'stock_owner'::regrole
          AND (c.relname = 'Member' OR c.relname LIKE 'member\_%' OR c.relname = 'notify_topic')
    LOOP
        EXECUTE format('ALTER %s %s OWNER TO stock_owner',
                       CASE r.relkind WHEN 'v' THEN 'VIEW' WHEN 'm' THEN 'MATERIALIZED VIEW' ELSE 'TABLE' END,
                       r.obj);
    END LOOP;

    -- 結構檔建立的觸發器函式
    FOR r IN
        SELECT p.oid::regprocedure AS obj
        FROM pg_proc AS p
        WHERE p.pronamespace = 'public'::regnamespace
          AND p.proowner <> 'stock_owner'::regrole
          AND p.proname LIKE 'member\_%'
    LOOP
        EXECUTE format('ALTER FUNCTION %s OWNER TO stock_owner', r.obj);
    END LOOP;
END $$;

-- =============================================================================
-- 2. 撤銷：各服務角色在 IAM 表上的全部權限（表層級 REVOKE 會一併撤銷欄位層級權限）
-- =============================================================================
DO $$
DECLARE
    r record;
BEGIN
    FOR r IN
        SELECT c.oid::regclass AS obj
        FROM pg_class AS c
        WHERE c.relnamespace = 'public'::regnamespace
          AND c.relkind IN ('r', 'p', 'v', 'm')
          AND (c.relname = 'Member' OR c.relname LIKE 'member\_%' OR c.relname = 'notify_topic')
    LOOP
        EXECUTE format('REVOKE ALL ON %s FROM stock_rust_app, stock_go_app, stock_go_maint, stock_readonly', r.obj);
    END LOOP;
END $$;
REVOKE ALL ON SEQUENCE public."Member_Id_seq" FROM stock_rust_app, stock_go_app, stock_go_maint, stock_readonly;

-- =============================================================================
-- 3. stock_rust_app：只讀分送所需的欄位；不碰密碼雜湊、Email、MFA、Session、Token、群組、稽核與訂閱
-- =============================================================================
-- "CreateTime"：每日結算只為建立日不晚於結算日的會員產生市值列（daily_money_history_member.rs）
GRANT SELECT ("Id", "Name", "CreateTime", household_id, is_active, deleted_at, must_change_password, activated_at)
    ON public."Member" TO stock_rust_app;
GRANT SELECT ON public.member_household TO stock_rust_app;
-- 視圖以擁有者權限讀底層表，不必開放 member_notify_subscription／member_notify_channel 的讀取
GRANT SELECT ON public.member_notify_route TO stock_rust_app;
-- 啟動時依程式內的目錄 upsert 通知主題
GRANT SELECT, INSERT, UPDATE ON public.notify_topic TO stock_rust_app;
-- 送達紀錄：先佔位、再發送、後更新（7.6）
GRANT SELECT, INSERT, UPDATE ON public.member_notify_delivery TO stock_rust_app;
-- 送達失敗時停用管道或累計失敗次數；UPDATE … WHERE id = … SET fail_count = fail_count + 1
-- 需要讀得到條件與運算式用到的欄位，所以另給這幾欄的 SELECT（不含 address 與 member_id）
GRANT SELECT (id, enabled, disabled_reason, fail_count, last_ok_at),
      UPDATE (enabled, disabled_reason, fail_count, last_ok_at)
    ON public.member_notify_channel TO stock_rust_app;
-- 重運算租約：以單句 UPDATE 原子消耗（started_at）並在收尾寫 finished_at（2.3）
GRANT SELECT, UPDATE (started_at, finished_at) ON public.member_analysis_lease TO stock_rust_app;

-- =============================================================================
-- 4. stock_go_app：IAM 表讀寫；稽核紀錄只能新增與查詢
-- =============================================================================
DO $$
DECLARE
    r record;
BEGIN
    FOR r IN
        SELECT c.oid::regclass AS obj
        FROM pg_class AS c
        WHERE c.relnamespace = 'public'::regnamespace
          AND c.relkind IN ('r', 'p')
          AND (c.relname = 'Member' OR c.relname LIKE 'member\_%' OR c.relname = 'notify_topic')
          AND c.relname <> 'member_audit_log'
    LOOP
        EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON %s TO stock_go_app', r.obj);
    END LOOP;
END $$;
GRANT SELECT, INSERT ON public.member_audit_log TO stock_go_app;
GRANT SELECT ON public.member_notify_route TO stock_go_app;
GRANT USAGE, SELECT ON SEQUENCE public."Member_Id_seq" TO stock_go_app;

-- =============================================================================
-- 5. stock_go_maint：每日清理（4.4）與稽核去識別化（4.6）；API 請求路徑不得取得這個連線池
-- =============================================================================
-- 清理結果寫成 member_audit_log 的 maint.cleanup 事件，所以也要能新增
GRANT SELECT, INSERT, DELETE ON public.member_audit_log TO stock_go_maint;
GRANT UPDATE (actor_member_id, target_id, detail, ip, user_agent) ON public.member_audit_log TO stock_go_maint;
-- 每 10 分鐘對帳：pending 超過 30 分鐘改為 unknown
GRANT SELECT, DELETE, UPDATE (status) ON public.member_notify_delivery TO stock_go_maint;
GRANT SELECT, DELETE ON public.member_session TO stock_go_maint;
GRANT SELECT, DELETE ON public.member_analysis_lease TO stock_go_maint;
GRANT SELECT, DELETE ON public.member_analysis_usage TO stock_go_maint;
GRANT SELECT, DELETE ON public.member_invitation TO stock_go_maint;
GRANT SELECT, DELETE ON public.member_email_challenge TO stock_go_maint;
GRANT SELECT, DELETE ON public.member_data_request TO stock_go_maint;
-- 已撤銷或過期的 Token 保留 1 年後清除
GRANT SELECT, DELETE ON public.member_mcp_token TO stock_go_maint;
-- 只允許執行有範圍的公開註冊清理，不能直接刪除會員或家庭。
REVOKE DELETE ON public."Member" FROM stock_go_maint;
REVOKE SELECT ("Id", activated_at, "CreateTime") ON public."Member" FROM stock_go_maint;
GRANT EXECUTE ON FUNCTION public.member_cleanup_unactivated_registrations(timestamptz) TO stock_go_maint;
-- 資料權利執行與重放使用獨立維護角色，只透過固定範圍的函式寫入。
REVOKE INSERT, UPDATE, DELETE ON public.member_privacy_epoch FROM stock_go_app;
GRANT EXECUTE ON FUNCTION public.member_prepare_data_delete(uuid) TO stock_go_maint;
GRANT EXECUTE ON FUNCTION public.member_execute_data_delete(uuid,bigint,timestamptz,boolean) TO stock_go_maint;
-- 資料權利執行工具（4.6，Phase 7）另需的權限在 Phase 7 補上，這裡只涵蓋到期清理
