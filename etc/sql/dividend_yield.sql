-- DividendYield：舊版年度殖利率表（PascalCase 欄位；早期 stock_go 建立）。目前 stock_rust、stock_go、stock_api 皆未引用，保留結構以便重建正式庫。
-- 結構由 2026-10-19 正式庫 pg_dump --schema-only 導出。

CREATE TABLE public."DividendYield" (
    "Serial" bigint NOT NULL,
    "SecurityCode" character varying(24) DEFAULT ''::character varying NOT NULL,
    "Year" bigint DEFAULT 0 NOT NULL,
    "Cash" numeric(18,4) DEFAULT 0 NOT NULL,
    "Stock" numeric(18,4) DEFAULT 0 NOT NULL,
    "Sum" numeric(18,4) DEFAULT 0 NOT NULL,
    "Highest" numeric(18,4) DEFAULT 0 NOT NULL,
    "Lowest" numeric(18,4) DEFAULT 0 NOT NULL,
    "Average" numeric(18,4) DEFAULT 0 NOT NULL,
    "CreateTime" timestamp with time zone DEFAULT ('now'::text)::timestamp with time zone NOT NULL
);

COMMENT ON COLUMN public."DividendYield"."Cash" IS '現金(%)';

COMMENT ON COLUMN public."DividendYield"."Stock" IS '股票(%)';

COMMENT ON COLUMN public."DividendYield"."Sum" IS '合計(%)';

COMMENT ON COLUMN public."DividendYield"."Highest" IS '最高股價';

COMMENT ON COLUMN public."DividendYield"."Lowest" IS '最低設價';

COMMENT ON COLUMN public."DividendYield"."Average" IS '平均股價';

CREATE SEQUENCE public."DividendYield_Serial_seq"
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public."DividendYield_Serial_seq" OWNED BY public."DividendYield"."Serial";

ALTER TABLE ONLY public."DividendYield" ALTER COLUMN "Serial" SET DEFAULT nextval('public."DividendYield_Serial_seq"'::regclass);

ALTER TABLE ONLY public."DividendYield"
    ADD CONSTRAINT "DividendYield_pkey" PRIMARY KEY ("SecurityCode", "Year");
