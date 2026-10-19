-- market_category：市場分類代碼表（上市／上櫃／興櫃）；stock_go 的 internal/database 仍有查詢。
-- 結構由 2026-10-19 正式庫 pg_dump --schema-only 導出。

CREATE TABLE public.market_category (
    market_category_id integer DEFAULT 0 NOT NULL,
    exchange character varying(32) DEFAULT ''::character varying NOT NULL,
    name character varying(32) DEFAULT ''::character varying NOT NULL
);

COMMENT ON COLUMN public.market_category.market_category_id IS '市場分類編號';

COMMENT ON COLUMN public.market_category.exchange IS '市場別 TAI:上市 TWO:上櫃 TWE:興櫃';

COMMENT ON COLUMN public.market_category.name IS '市場名稱';

ALTER TABLE ONLY public.market_category
    ADD CONSTRAINT market_category_pkey PRIMARY KEY (market_category_id);

INSERT INTO public.market_category (market_category_id, exchange, name) VALUES
    (2, 'TAI', '上市'),
    (4, 'TWO', '上櫃'),
    (5, 'TWE', '興櫃');
