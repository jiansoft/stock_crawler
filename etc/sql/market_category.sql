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

CREATE TABLE IF NOT EXISTS stocks
(
    "SecurityCode"            VARCHAR(24)              DEFAULT ''::CHARACTER VARYING                   NOT NULL,
    "Name"                    VARCHAR(255)             DEFAULT ''::CHARACTER VARYING                   NOT NULL,
    "CreateTime"              TIMESTAMP WITH TIME ZONE DEFAULT ('now'::TEXT)::TIMESTAMP WITH TIME ZONE NOT NULL,
    "SuspendListing"          BOOLEAN                  DEFAULT FALSE                                   NOT NULL,
    last_one_eps              NUMERIC(18, 4)           DEFAULT 0                                       NOT NULL,
    last_four_eps             NUMERIC(18, 4)           DEFAULT 0                                       NOT NULL,
    return_on_equity          NUMERIC(18, 4)           DEFAULT 0                                       NOT NULL,
    net_asset_value_per_share NUMERIC(18, 4)           DEFAULT 0                                       NOT NULL,
    stock_symbol              VARCHAR(24)              DEFAULT ''::CHARACTER VARYING                   NOT NULL PRIMARY KEY,
    stock_exchange_market_id  INT4                                                                     NOT NULL DEFAULT 0,
    stock_industry_id INT4                     NOT NULL DEFAULT 0,
    weight                        NUMERIC(18, 4) DEFAULT 0 NOT NULL,
    issued_share                  INT8           DEFAULT 0 NOT NULL,
    qfii_shares_held              INT8           DEFAULT 0 NOT NULL,
    qfii_share_holding_percentage NUMERIC(18, 4) DEFAULT 0 NOT NULL
    );


COMMENT ON COLUMN stocks.last_four_eps IS '近四季EPS';
COMMENT ON COLUMN stocks.last_one_eps IS '近一季EPS';
COMMENT ON COLUMN stocks.net_asset_value_per_share IS '每股淨值';
COMMENT ON COLUMN stocks.stock_symbol IS '股票代號同 SecurityCode';
COMMENT ON COLUMN stocks.stock_exchange_market_id IS '交易所的市場編號參考 stock_exchange_market';
COMMENT ON COLUMN stocks.stock_industry_id IS '股票的產業分類編號 stock_industry';
COMMENT ON COLUMN stocks.return_on_equity IS '股東權益報酬率';
COMMENT ON COLUMN stocks.weight IS '權值佔比';
COMMENT ON COLUMN stocks.return_on_equity IS '股東權益報酬率';
COMMENT ON COLUMN stocks.issued_share IS '發行股數';
COMMENT ON COLUMN stocks.qfii_shares_held IS '全體外資及陸資持有股數';
COMMENT ON COLUMN stocks.qfii_share_holding_percentage IS '全體外資及陸資持股比率';

CREATE INDEX "stocks-stock_exchange_market_id-stock_industry_id-idx" ON stocks USING BTREE (stock_exchange_market_id, stock_industry_id);
CREATE INDEX "stocks-stock_industry_id-idx" ON stocks USING BTREE (stock_industry_id);