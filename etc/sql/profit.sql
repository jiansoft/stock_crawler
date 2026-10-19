-- profit：舊版季度／年度獲利指標表（現行財報改用 financial_statement）。目前 stock_rust、stock_go、stock_api 皆未引用，保留結構以便重建正式庫。
-- 結構由 2026-10-19 正式庫 pg_dump --schema-only 導出。

CREATE TABLE public.profit (
    security_code character varying(24) DEFAULT ''::character varying NOT NULL,
    year bigint DEFAULT 0 NOT NULL,
    quarter character varying(4) DEFAULT ''::character varying NOT NULL,
    gross_profit numeric(18,4) DEFAULT 0 NOT NULL,
    operating_profit_margin numeric(18,4) DEFAULT 0 NOT NULL,
    "pre-tax_income" numeric(18,4) DEFAULT 0 NOT NULL,
    net_income numeric(18,4) DEFAULT 0 NOT NULL,
    net_asset_value_per_share numeric(18,4) DEFAULT 0 NOT NULL,
    sales_per_share numeric(18,4) DEFAULT 0 NOT NULL,
    earnings_per_share numeric(18,4) DEFAULT 0 NOT NULL,
    profit_before_tax numeric(18,4) DEFAULT 0 NOT NULL,
    return_on_equity numeric(18,4) DEFAULT 0 NOT NULL,
    return_on_assets numeric(18,4) DEFAULT 0 NOT NULL,
    created_time timestamp(6) without time zone DEFAULT ('now'::text)::timestamp without time zone NOT NULL,
    updated_time timestamp(6) without time zone DEFAULT ('now'::text)::timestamp without time zone NOT NULL
);

COMMENT ON COLUMN public.profit.year IS '年度';

COMMENT ON COLUMN public.profit.quarter IS '季度 A:全年度 Q1~Q4:第一季~第四季';

COMMENT ON COLUMN public.profit.gross_profit IS '營業毛利率';

COMMENT ON COLUMN public.profit.operating_profit_margin IS '營業利益率';

COMMENT ON COLUMN public.profit."pre-tax_income" IS '稅前淨利率';

COMMENT ON COLUMN public.profit.net_income IS '稅後淨利率';

COMMENT ON COLUMN public.profit.net_asset_value_per_share IS '每股淨值';

COMMENT ON COLUMN public.profit.sales_per_share IS '每股營收';

COMMENT ON COLUMN public.profit.earnings_per_share IS '每股稅後淨利';

COMMENT ON COLUMN public.profit.profit_before_tax IS '每股稅前淨利';

COMMENT ON COLUMN public.profit.return_on_equity IS '股東權益報酬率';

COMMENT ON COLUMN public.profit.return_on_assets IS '資產報酬率';

ALTER TABLE ONLY public.profit
    ADD CONSTRAINT profit_pkey PRIMARY KEY (security_code, year, quarter);

CREATE INDEX "idx-profit-year-quarter" ON public.profit USING btree (year, quarter);
