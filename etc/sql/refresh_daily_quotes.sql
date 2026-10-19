-- RefreshDailyQuotes：舊版收盤資料刷新紀錄表（PascalCase 欄位；早期 stock_go 建立）。目前 stock_rust、stock_go、stock_api 皆未引用，保留結構以便重建正式庫。
-- 結構由 2026-10-19 正式庫 pg_dump --schema-only 導出。

CREATE TABLE public."RefreshDailyQuotes" (
    "Serial" bigint NOT NULL,
    "Type" integer DEFAULT 0 NOT NULL,
    "Date" date DEFAULT CURRENT_DATE NOT NULL,
    "CreateTime" timestamp with time zone DEFAULT ('now'::text)::timestamp with time zone NOT NULL
);

COMMENT ON COLUMN public."RefreshDailyQuotes"."Type" IS '1:收盤資料';

CREATE SEQUENCE public."RefreshDailyQuotes_Serial_seq"
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public."RefreshDailyQuotes_Serial_seq" OWNED BY public."RefreshDailyQuotes"."Serial";

ALTER TABLE ONLY public."RefreshDailyQuotes" ALTER COLUMN "Serial" SET DEFAULT nextval('public."RefreshDailyQuotes_Serial_seq"'::regclass);

ALTER TABLE ONLY public."RefreshDailyQuotes"
    ADD CONSTRAINT "RefreshDailyQuotes_pkey" PRIMARY KEY ("Serial");
