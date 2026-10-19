-- backfill_requests：stock_go 手動回補的持久化提交收據（防重複提交）。由 stock_go 擁有（來源：stock_go 倉庫 docs/sql/operations.sql，2026-10-19 R3 部署時建立）；stock_rust 不讀寫此表，這裡只保留結構以便重建正式庫。
-- 結構由 2026-10-19 正式庫 pg_dump --schema-only 導出。

CREATE TABLE public.backfill_requests (
    request_key character varying(100) NOT NULL,
    payload_hash character(64) NOT NULL,
    operation character varying(64) NOT NULL,
    state character varying(16) NOT NULL,
    http_status integer DEFAULT 0 NOT NULL,
    response_body bytea,
    job_id text DEFAULT ''::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT backfill_requests_state_check CHECK (((state)::text = ANY ((ARRAY['submitting'::character varying, 'submitted'::character varying, 'rejected'::character varying, 'unknown'::character varying])::text[])))
);

ALTER TABLE ONLY public.backfill_requests
    ADD CONSTRAINT backfill_requests_pkey PRIMARY KEY (request_key);

CREATE INDEX backfill_requests_created_idx ON public.backfill_requests USING btree (created_at DESC);
