-- quality_alert_events：stock_go 資料品質提醒事件（rule_id 參考 quality_alert_rules，須先建該表）。由 stock_go 擁有（來源：stock_go 倉庫 docs/sql/operations.sql，2026-10-19 R3 部署時建立）；stock_rust 不讀寫此表，這裡只保留結構以便重建正式庫。
-- 結構由 2026-10-19 正式庫 pg_dump --schema-only 導出。

CREATE TABLE public.quality_alert_events (
    id bigint NOT NULL,
    rule_id bigint NOT NULL,
    dataset character varying(16) NOT NULL,
    stock_symbol character varying(20) DEFAULT ''::character varying NOT NULL,
    reason character varying(24) NOT NULL,
    observed_value bigint,
    data_date text,
    opened_at timestamp with time zone NOT NULL,
    last_seen_at timestamp with time zone NOT NULL,
    resolved_at timestamp with time zone,
    resolution text DEFAULT ''::text NOT NULL
);

CREATE SEQUENCE public.quality_alert_events_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.quality_alert_events_id_seq OWNED BY public.quality_alert_events.id;

ALTER TABLE ONLY public.quality_alert_events ALTER COLUMN id SET DEFAULT nextval('public.quality_alert_events_id_seq'::regclass);

ALTER TABLE ONLY public.quality_alert_events
    ADD CONSTRAINT quality_alert_events_pkey PRIMARY KEY (id);

CREATE UNIQUE INDEX quality_alert_events_active_idx ON public.quality_alert_events USING btree (rule_id) WHERE (resolved_at IS NULL);

CREATE INDEX quality_alert_events_opened_idx ON public.quality_alert_events USING btree (opened_at DESC);

ALTER TABLE ONLY public.quality_alert_events
    ADD CONSTRAINT quality_alert_events_rule_id_fkey FOREIGN KEY (rule_id) REFERENCES public.quality_alert_rules(id);
