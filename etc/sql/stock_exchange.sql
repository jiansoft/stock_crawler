-- stock_exchange：交易所代碼表（TWSE／TPEx），stock_exchange_market.stock_exchange_id 參考此表。
-- 結構由 2026-10-19 正式庫 pg_dump --schema-only 導出。

CREATE TABLE public.stock_exchange (
    stock_exchange_id integer DEFAULT 0 NOT NULL,
    code character varying(24) DEFAULT ''::character varying NOT NULL,
    name character varying(24) DEFAULT ''::character varying NOT NULL
);

COMMENT ON COLUMN public.stock_exchange.stock_exchange_id IS '市場分類編號';

COMMENT ON COLUMN public.stock_exchange.code IS '市場代號 TWSE:臺灣證券交易所 TPEx:證券櫃檯買賣市場';

COMMENT ON COLUMN public.stock_exchange.name IS '市場名稱';

ALTER TABLE ONLY public.stock_exchange
    ADD CONSTRAINT stock_exchange_pkey PRIMARY KEY (stock_exchange_id);

INSERT INTO public.stock_exchange (stock_exchange_id, code, name) VALUES
    (1, 'TWSE', '臺灣證券交易所'),
    (2, 'TPEx', '證券櫃檯買賣市場');
