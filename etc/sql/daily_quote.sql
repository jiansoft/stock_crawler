-- DROP TABLE IF EXISTS "public"."DailyQuotes";
CREATE TABLE "DailyQuotes"
(
    "Serial"                        BIGSERIAL                                     NOT NULL
        CONSTRAINT "DailyQuotes_pkey" PRIMARY KEY,
    "Date"                          DATE                     DEFAULT CURRENT_DATE NOT NULL,
    "year"                          INTEGER                  DEFAULT 0            NOT NULL,
    "month"                         INTEGER                  DEFAULT 0            NOT NULL,
    "day"                           INTEGER                  DEFAULT 0            NOT NULL,
    "SecurityCode"                  VARCHAR(24)              DEFAULT ''           NOT NULL,
    "TradingVolume"                 NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "Transaction"                   NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "TradeValue"                    NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "OpeningPrice"                  NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "HighestPrice"                  NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "LowestPrice"                   NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "ClosingPrice"                  NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "ChangeRange"                   NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "Change"                        NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "LastBestBidPrice"              NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "LastBestBidVolume"             NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "LastBestAskPrice"              NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "LastBestAskVolume"             NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "PriceEarningRatio"             NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "MovingAverage5"                NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "MovingAverage10"               NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "MovingAverage20"               NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "MovingAverage60"               NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "MovingAverage120"              NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "MovingAverage240"              NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "maximum_price_in_year"         NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "minimum_price_in_year"         NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "average_price_in_year"         NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "price-to-book_ratio"           NUMERIC(18, 4)           DEFAULT 0            NOT NULL,
    "maximum_price_in_year_date_on" DATE                     DEFAULT '1970-01-01' NOT NULL,
    "minimum_price_in_year_date_on" DATE                     DEFAULT '1970-01-01' NOT NULL,
    "RecordTime"                    TIMESTAMP WITH TIME ZONE DEFAULT NOW()        NOT NULL,
    "CreateTime"                    TIMESTAMP WITH TIME ZONE DEFAULT NOW()        NOT NULL
);

COMMENT ON COLUMN "DailyQuotes"."Date" IS '資料屬於那一天(收盤日)';
COMMENT ON COLUMN "DailyQuotes"."SecurityCode" IS '股票代碼';
COMMENT ON COLUMN "DailyQuotes"."TradingVolume" IS '成交股數';
COMMENT ON COLUMN "DailyQuotes"."Transaction" IS '成交筆數';
COMMENT ON COLUMN "DailyQuotes"."TradeValue" IS '成交金額';
COMMENT ON COLUMN "DailyQuotes"."OpeningPrice" IS '開盤價';
COMMENT ON COLUMN "DailyQuotes"."HighestPrice" IS '最高價';
COMMENT ON COLUMN "DailyQuotes"."LowestPrice" IS '最低價';
COMMENT ON COLUMN "DailyQuotes"."ClosingPrice" IS '收盤價';
COMMENT ON COLUMN "DailyQuotes"."ChangeRange" IS '漲幅';
COMMENT ON COLUMN "DailyQuotes"."Change" IS '漲跌價差';
COMMENT ON COLUMN "DailyQuotes"."LastBestBidPrice" IS '最後揭示買價';
COMMENT ON COLUMN "DailyQuotes"."LastBestBidVolume" IS '最後揭示買量';
COMMENT ON COLUMN "DailyQuotes"."LastBestAskPrice" IS '最後揭示賣價';
COMMENT ON COLUMN "DailyQuotes"."LastBestAskVolume" IS '最後揭示賣量';
COMMENT ON COLUMN "DailyQuotes"."PriceEarningRatio" IS '本益比';
COMMENT ON COLUMN "DailyQuotes"."RecordTime" IS '資料日期';
COMMENT ON COLUMN "DailyQuotes".year IS '資料屬於那年度';
COMMENT ON COLUMN "DailyQuotes".month IS '資料屬於那月份';
COMMENT ON COLUMN "DailyQuotes".day IS '資料屬於那日';
COMMENT ON COLUMN "DailyQuotes"."MovingAverage5" IS '5日週線';
COMMENT ON COLUMN "DailyQuotes"."MovingAverage10" IS '10日雙週線';
COMMENT ON COLUMN "DailyQuotes"."MovingAverage20" IS '20日月線';
COMMENT ON COLUMN "DailyQuotes"."MovingAverage60" IS '60日季線';
COMMENT ON COLUMN "DailyQuotes"."MovingAverage120" IS '120日半年線';
COMMENT ON COLUMN "DailyQuotes"."MovingAverage240" IS '240日年線';
COMMENT ON COLUMN "DailyQuotes"."maximum_price_in_year" IS '一年內最高價(收盤日為起點)';
COMMENT ON COLUMN "DailyQuotes"."minimum_price_in_year" IS '一年內最低價(收盤日為起點)';
COMMENT ON COLUMN "DailyQuotes"."average_price_in_year" IS '一年內平均價(收盤日為起點)';
COMMENT ON COLUMN "DailyQuotes"."maximum_price_in_year_date_on" IS '一年內最高價在哪一天(收盤日為起點)';
COMMENT ON COLUMN "DailyQuotes"."minimum_price_in_year_date_on" IS '一年內最低價在哪一天(收盤日為起點)';
COMMENT ON COLUMN "DailyQuotes"."price-to-book_ratio" IS '股價淨值比';

CREATE UNIQUE INDEX ON "DailyQuotes" ("Date" DESC, "SecurityCode" ASC);

DROP INDEX "DailyQuotes_SecurityCode_Date_uidx";
CREATE UNIQUE INDEX "DailyQuotes_SecurityCode_Date_uidx"
    ON "DailyQuotes" ("SecurityCode", "Date" DESC) INCLUDE ("year", "HighestPrice", "LowestPrice", "ClosingPrice");

DROP INDEX "DailyQuotes_Date_idx";
CREATE INDEX "DailyQuotes_Date_idx"
    ON "DailyQuotes" ("Date" DESC) INCLUDE ("Serial", "SecurityCode");
