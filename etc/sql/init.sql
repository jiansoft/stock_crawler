DROP TABLE IF EXISTS "public"."market_category";
CREATE TABLE "public"."market_category"
(
    "market_category_id" int4                                      NOT NULL default 0,
    "exchange"           varchar(32) default ''::character varying not null,
    "name"               varchar(32) default ''::character varying not null,
    CONSTRAINT "market_category_pkey" PRIMARY KEY (market_category_id)

);

COMMENT ON COLUMN "public"."market_category"."exchange" IS '市場別 TAI:上市 TWO:上櫃 TWE:興櫃';
COMMENT ON COLUMN "public"."market_category"."market_category_id" IS '市場分類編號';
COMMENT ON COLUMN "public"."market_category"."name" IS '市場名稱';

insert into public.market_category (exchange, market_category_id, name)
values ('TAI', 2, '上市'),
       ('TWO', 4, '上櫃'),
       ('TWE', 5, '興櫃');