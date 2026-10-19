-- Dividend：舊版年度股利表（PascalCase 欄位；早期 stock_go 建立，現行股利改用 dividend 表，見 dividend.sql）。目前 stock_rust、stock_go、stock_api 皆未引用，保留結構以便重建正式庫。
-- 結構由 2026-10-19 正式庫 pg_dump --schema-only 導出。

CREATE TABLE public."Dividend" (
    "Serial" bigint NOT NULL,
    "SecurityCode" character varying(24) DEFAULT ''::character varying NOT NULL,
    "Year" bigint DEFAULT 0 NOT NULL,
    "CashDividend" numeric(18,4) DEFAULT 0 NOT NULL,
    "StockDividend" numeric(18,4) DEFAULT 0 NOT NULL,
    "RetainedEarnings" numeric(18,4) DEFAULT 0 NOT NULL,
    "CapitalReserve" numeric(18,4) DEFAULT 0 NOT NULL,
    "Sum" numeric(18,4) DEFAULT 0 NOT NULL,
    "CreateTime" timestamp with time zone DEFAULT ('now'::text)::timestamp with time zone NOT NULL
);

COMMENT ON COLUMN public."Dividend"."CashDividend" IS '現金股利';

COMMENT ON COLUMN public."Dividend"."StockDividend" IS '股票股利';

COMMENT ON COLUMN public."Dividend"."RetainedEarnings" IS '盈餘配股';

COMMENT ON COLUMN public."Dividend"."CapitalReserve" IS '公積配股';

COMMENT ON COLUMN public."Dividend"."Sum" IS '合計';

CREATE SEQUENCE public."Dividend_Serial_seq"
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public."Dividend_Serial_seq" OWNED BY public."Dividend"."Serial";

ALTER TABLE ONLY public."Dividend" ALTER COLUMN "Serial" SET DEFAULT nextval('public."Dividend_Serial_seq"'::regclass);

ALTER TABLE ONLY public."Dividend"
    ADD CONSTRAINT "Dividend_pkey" PRIMARY KEY ("SecurityCode", "Year");

CREATE INDEX "idx-Dividend-Serial" ON public."Dividend" USING btree ("Serial");

CREATE INDEX "idx-Dividend-Year" ON public."Dividend" USING btree ("Year");
