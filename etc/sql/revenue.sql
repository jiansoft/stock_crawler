create table public."Revenue"
(
    "Serial"                          bigserial
        constraint "Revenue_pkey" primary key,
    "SecurityCode"                    varchar(24)              default ''::character varying                   not null,
    "Date"                            bigint                   default 0                                       not null,
    "Monthly"                         numeric(18, 4)           default 0                                       not null,
    "LastMonth"                       numeric(18, 4)           default 0                                       not null,
    "LastYearThisMonth"               numeric(18, 4)           default 0                                       not null,
    "MonthlyAccumulated"              numeric(18, 4)           default 0                                       not null,
    "LastYearMonthlyAccumulated"      numeric(18, 4)           default 0                                       not null,
    "ComparedWithLastMonth"           numeric(18, 4)           default 0                                       not null,
    "ComparedWithLastYearSameMonth"   numeric(18, 4)           default 0                                       not null,
    "AccumulatedComparedWithLastYear" numeric(18, 4)           default 0                                       not null,
    "CreateTime"                      timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    avg_price                         numeric(18, 4)           default 0                                       not null,
    lowest_price                      numeric(18, 4)           default 0                                       not null,
    highest_price                     numeric(18, 4)           default 0                                       not null,
    stock_symbol                      varchar(24)              default ''::character varying                   not null
);

comment on column public."Revenue"."Monthly" is '當月營收';
comment on column public."Revenue"."LastMonth" is '上月營收';
comment on column public."Revenue"."LastYearThisMonth" is '去年當月營收';
comment on column public."Revenue"."MonthlyAccumulated" is '當月累計營收';
comment on column public."Revenue"."LastYearMonthlyAccumulated" is '去年累計營收';
comment on column public."Revenue"."ComparedWithLastMonth" is '上月比較增減(%)';
comment on column public."Revenue"."ComparedWithLastYearSameMonth" is '去年同月增減(%)';
comment on column public."Revenue"."AccumulatedComparedWithLastYear" is '前期比較增減(%)';
comment on column public."Revenue".avg_price is '月均價';
comment on column public."Revenue".lowest_price is '當月最低價';
comment on column public."Revenue".highest_price is '當月最高價';
comment on column public."Revenue"."stock_symbol" is '股票代碼';

create unique index "Revenue_SecurityCode_Date-uidx"
    on public."Revenue" ("SecurityCode", "Date");


create unique index "Revenue_stock_symbol_Date_uidx" on public."Revenue" ("stock_symbol" asc, "Date" desc)


