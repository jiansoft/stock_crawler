create table public.dividend
(
    serial                         bigserial,
    security_code                  varchar(24)              default ''::character varying                   not null,
    year                           integer                  default 0                                       not null,
    year_of_dividend               integer                  default 0                                       not null,
    quarter                        varchar(4)               default ''::character varying                   not null,
    cash_dividend                  numeric(18, 4)           default 0                                       not null,
    stock_dividend                 numeric(18, 4)           default 0                                       not null,
    sum                            numeric(18, 4)           default 0                                       not null,
    "ex-dividend_date1"            varchar(10)              default ''::character varying                   not null,
    "ex-dividend_date2"            varchar(10)              default ''::character varying                   not null,
    payable_date1                  varchar(10)              default ''::character varying                   not null,
    payable_date2                  varchar(10)              default ''::character varying                   not null,
    created_time                   timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    updated_time                   timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    capital_reserve_cash_dividend  numeric(18, 4)           default 0                                       not null,
    earnings_cash_dividend         numeric(18, 4)           default 0                                       not null,
    capital_reserve_stock_dividend numeric(18, 4)           default 0                                       not null,
    earnings_stock_dividend        numeric(18, 4)           default 0                                       not null,
    payout_ratio_cash              numeric(18, 4)           default 0                                       not null,
    payout_ratio_stock             numeric(18, 4)           default 0                                       not null,
    payout_ratio                   numeric(18, 4)           default 0                                       not null,
    payout_eps                     numeric(18, 4),
    payout_period                  varchar(16),
    -- 同一發放年度可能有兩次同期別的配息（大立光 2022 年同時發 2021H1 與 2022H1），
    -- 所屬年度必須是主鍵的一部分。
    primary key (security_code, year, year_of_dividend, quarter)
);

-- 年度層級列（quarter = ''：單次年配本身，或多次配發的年度合計）每個發放年度只能有一列，
-- 估價、殖利率排行與 stock_go 都依此取該年度的股利總額。
create unique index dividend_annual_level_uidx on public.dividend (security_code, year) where quarter = '';

comment on column public.dividend.year_of_dividend is '股利所屬年度';
comment on column public.dividend.quarter is '季度 空字串:年度（單次年配或年度合計） A:混合配息年度的全年事件 Q1~Q4:第一季~第四季 H1~H2:上半年~下半年 M01~M12:月配';
comment on column public.dividend.cash_dividend is '現金股利';
comment on column public.dividend.stock_dividend is '股票股利';
comment on column public.dividend.sum is '合計';
comment on column public.dividend."ex-dividend_date1" is '除息日';
comment on column public.dividend."ex-dividend_date2" is '除權日';
comment on column public.dividend.payable_date1 is '現金股利發放日';
comment on column public.dividend.payable_date2 is '股票股利發放日';
comment on column public.dividend.capital_reserve_cash_dividend is '公積現金股利';
comment on column public.dividend.earnings_cash_dividend is '盈餘現金股利';
comment on column public.dividend.capital_reserve_stock_dividend is '公積股票股利';
comment on column public.dividend.earnings_stock_dividend is '盈餘股票股利';
comment on column public.dividend.payout_ratio_cash is '盈餘分配率_現金(%)';
comment on column public.dividend.payout_ratio_stock is '盈餘分配率_股要(%)';
comment on column public.dividend.payout_ratio is '盈餘分配率(%)';
comment on column public.dividend.payout_eps is '盈餘分配率的分母：這筆股利涵蓋期間的每股盈餘（尚未計算時為 NULL）';
comment on column public.dividend.payout_period is '股利涵蓋的盈餘期間，例如 2025、2026Q2、2025Q4~2026Q2（尚未計算時為 NULL）';

create index "dividend-serial-idx"
    on public.dividend (serial);

create index "dividend-year-dividend_date-idx"
    on public.dividend (year, "ex-dividend_date1", "ex-dividend_date2");

create index "dividend-year-payable_date-idx"
    on public.dividend (year, payable_date1, payable_date2);

