-- quality_alert_rules：stock_go 資料品質提醒規則。由 stock_go 擁有（來源：stock_go 倉庫 docs/sql/operations.sql，2026-10-19 R3 部署時建立）；stock_rust 不讀寫此表，這裡只保留結構以便重建正式庫。
-- 結構由 2026-10-19 正式庫 pg_dump --schema-only 導出。

CREATE TABLE public.quality_alert_rules (
    id bigint NOT NULL,
    dataset character varying(16) NOT NULL,
    stock_symbol character varying(20) DEFAULT ''::character varying NOT NULL,
    kind character varying(20) NOT NULL,
    threshold integer NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    version bigint DEFAULT 1 NOT NULL,
    deleted_at timestamp with time zone,
    last_evaluated_at timestamp with time zone,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT quality_alert_rules_threshold_check CHECK (((threshold >= 0) AND (threshold <= 3660)))
);

CREATE SEQUENCE public.quality_alert_rules_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public.quality_alert_rules_id_seq OWNED BY public.quality_alert_rules.id;

ALTER TABLE ONLY public.quality_alert_rules ALTER COLUMN id SET DEFAULT nextval('public.quality_alert_rules_id_seq'::regclass);

ALTER TABLE ONLY public.quality_alert_rules
    ADD CONSTRAINT quality_alert_rules_dataset_stock_symbol_kind_key UNIQUE (dataset, stock_symbol, kind);

ALTER TABLE ONLY public.quality_alert_rules
    ADD CONSTRAINT quality_alert_rules_pkey PRIMARY KEY (id);
