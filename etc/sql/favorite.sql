-- Favorite：舊版會員持股表（PascalCase 欄位，MemberId 參考 Member.Id；現行持股改用 stock_ownership_details）。目前 stock_rust、stock_go、stock_api 皆未引用，保留結構以便重建正式庫。
-- 結構由 2026-10-19 正式庫 pg_dump --schema-only 導出。

CREATE TABLE public."Favorite" (
    "Id" bigint NOT NULL,
    "MemberId" bigint DEFAULT 0 NOT NULL,
    "SecurityCode" character varying(24) DEFAULT ''::character varying NOT NULL,
    "NumberOfSharesHeld" bigint DEFAULT 0 NOT NULL,
    "AverageCost" numeric(18,4) DEFAULT 0 NOT NULL,
    "CreateTime" timestamp with time zone DEFAULT ('now'::text)::timestamp with time zone NOT NULL,
    "AmountPerShare" numeric(18,4) DEFAULT 0 NOT NULL,
    "IsSold" boolean DEFAULT false,
    cumulate_dividends_cash numeric(18,4) DEFAULT 0,
    cumulate_dividends_stock numeric(18,4) DEFAULT 0,
    cumulate_dividends_stock_money numeric(18,4) DEFAULT 0,
    cumulate_dividends_total numeric(18,4) DEFAULT 0
);

COMMENT ON COLUMN public."Favorite"."MemberId" IS '會員編號 Member.Id';

COMMENT ON COLUMN public."Favorite"."SecurityCode" IS '股票代碼';

COMMENT ON COLUMN public."Favorite"."NumberOfSharesHeld" IS '持有股數';

COMMENT ON COLUMN public."Favorite"."AverageCost" IS '平均成本';

COMMENT ON COLUMN public."Favorite"."AmountPerShare" IS '每股金額';

COMMENT ON COLUMN public."Favorite"."IsSold" IS '是否賣出';

COMMENT ON COLUMN public."Favorite".cumulate_dividends_cash IS '累積現金股利(元)';

COMMENT ON COLUMN public."Favorite".cumulate_dividends_stock IS '累積股票股利(股)';

COMMENT ON COLUMN public."Favorite".cumulate_dividends_stock_money IS '累積股票股利(元)';

COMMENT ON COLUMN public."Favorite".cumulate_dividends_total IS '總計累積股利(元)';

CREATE SEQUENCE public."Favorite_Id_seq"
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public."Favorite_Id_seq" OWNED BY public."Favorite"."Id";

ALTER TABLE ONLY public."Favorite" ALTER COLUMN "Id" SET DEFAULT nextval('public."Favorite_Id_seq"'::regclass);

ALTER TABLE ONLY public."Favorite"
    ADD CONSTRAINT "Favorite_pkey" PRIMARY KEY ("Id");

CREATE INDEX "Favorite_MemberId_SecurityCode" ON public."Favorite" USING btree ("MemberId", "SecurityCode");

CREATE INDEX "idx-Favorite-MemberId" ON public."Favorite" USING btree ("MemberId");
