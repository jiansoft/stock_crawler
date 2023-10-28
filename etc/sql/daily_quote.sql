-- DROP TABLE IF EXISTS "public"."DailyQuotes";
create table public."DailyQuotes"
(
    "Serial"                      bigserial
        primary key,
    "Date"                        date                     default CURRENT_DATE                            not null,
    "SecurityCode"                varchar(24)              default ''::character varying                   not null,
    "TradingVolume"               numeric(18, 4)           default 0                                       not null,
    "Transaction"                 numeric(18, 4)           default 0                                       not null,
    "TradeValue"                  numeric(18, 4)           default 0                                       not null,
    "OpeningPrice"                numeric(18, 4)           default 0                                       not null,
    "HighestPrice"                numeric(18, 4)           default 0                                       not null,
    "LowestPrice"                 numeric(18, 4)           default 0                                       not null,
    "ClosingPrice"                numeric(18, 4)           default 0                                       not null,
    "ChangeRange"                 numeric(18, 4)           default 0                                       not null,
    "Change"                      numeric(18, 4)           default 0                                       not null,
    "LastBestBidPrice"            numeric(18, 4)           default 0                                       not null,
    "LastBestBidVolume"           numeric(18, 4)           default 0                                       not null,
    "LastBestAskPrice"            numeric(18, 4)           default 0                                       not null,
    "LastBestAskVolume"           numeric(18, 4)           default 0                                       not null,
    "PriceEarningRatio"           numeric(18, 4)           default 0                                       not null,
    "RecordTime"                  timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    "CreateTime"                  timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    "MovingAverage5"              numeric(18, 4)           default 0                                       not null,
    "MovingAverage10"             numeric(18, 4)           default 0                                       not null,
    "MovingAverage20"             numeric(18, 4)           default 0                                       not null,
    "MovingAverage60"             numeric(18, 4)           default 0                                       not null,
    "MovingAverage120"            numeric(18, 4)           default 0                                       not null,
    "MovingAverage240"            numeric(18, 4)           default 0                                       not null,
    maximum_price_in_year         numeric(18, 4)           default 0                                       not null,
    minimum_price_in_year         numeric(18, 4)           default 0                                       not null,
    average_price_in_year         numeric(18, 4)           default 0                                       not null,
    maximum_price_in_year_date_on date                     default '1970-01-01'::date                      not null,
    minimum_price_in_year_date_on date                     default '1970-01-01'::date                      not null,
    "price-to-book_ratio"         numeric(18, 4)           default 0                                       not null,
    year                          integer                  default 0                                       not null,
    month                         integer                  default 0                                       not null,
    day                           integer                  default 0                                       not null
);

comment on column public."DailyQuotes"."Date" is '資料屬於那一天(收盤日)';
comment on column public."DailyQuotes"."SecurityCode" is '股票代碼';
comment on column public."DailyQuotes"."TradingVolume" is '成交股數';
comment on column public."DailyQuotes"."Transaction" is '成交筆數';
comment on column public."DailyQuotes"."TradeValue" is '成交金額';
comment on column public."DailyQuotes"."OpeningPrice" is '開盤價';
comment on column public."DailyQuotes"."HighestPrice" is '最高價';
comment on column public."DailyQuotes"."LowestPrice" is '最低價';
comment on column public."DailyQuotes"."ClosingPrice" is '收盤價';
comment on column public."DailyQuotes"."ChangeRange" is '漲幅';
comment on column public."DailyQuotes"."Change" is '漲跌價差';
comment on column public."DailyQuotes"."LastBestBidPrice" is '最後揭示買價';
comment on column public."DailyQuotes"."LastBestBidVolume" is '最後揭示買量';
comment on column public."DailyQuotes"."LastBestAskPrice" is '最後揭示賣價';
comment on column public."DailyQuotes"."LastBestAskVolume" is '最後揭示賣量';
comment on column public."DailyQuotes"."PriceEarningRatio" is '本益比';
comment on column public."DailyQuotes"."RecordTime" is '資料日期';
comment on column public."DailyQuotes"."MovingAverage5" is '5日週線';
comment on column public."DailyQuotes"."MovingAverage10" is '10日雙週線';
comment on column public."DailyQuotes"."MovingAverage20" is '20日月線';
comment on column public."DailyQuotes"."MovingAverage60" is '60日季線';
comment on column public."DailyQuotes"."MovingAverage120" is '120日半年線';
comment on column public."DailyQuotes"."MovingAverage240" is '240日年線';
comment on column public."DailyQuotes".maximum_price_in_year is '一年內最高價(收盤日為起點)';
comment on column public."DailyQuotes".minimum_price_in_year is '一年內最低價(收盤日為起點)';
comment on column public."DailyQuotes".average_price_in_year is '一年內平均價(收盤日為起點)';
comment on column public."DailyQuotes".maximum_price_in_year_date_on is '一年內最高價在哪一天(收盤日為起點)';
comment on column public."DailyQuotes".minimum_price_in_year_date_on is '一年內最低價在哪一天(收盤日為起點)';
comment on column public."DailyQuotes"."price-to-book_ratio" is '股價淨值比';
comment on column public."DailyQuotes".year is '資料屬於那一年度';
comment on column public."DailyQuotes".month is '資料屬於那月份';
comment on column public."DailyQuotes".day is '資料屬於那日';

create index "DailyQuotes_Date_idx"
    on public."DailyQuotes" ("Date" desc) include ("Serial", "SecurityCode");

create unique index "DailyQuotes_SecurityCode_Date_uidx"
    on public."DailyQuotes" ("SecurityCode" asc, "Date" desc) include (year, "HighestPrice", "LowestPrice", "ClosingPrice", "price-to-book_ratio", "PriceEarningRatio");


