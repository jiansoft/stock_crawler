create table public.financial_statement
(
    serial                    bigserial
        primary key,
    security_code             varchar(24)              default ''::character varying                   not null,
    year                      bigint                   default 0                                       not null,
    quarter                   varchar(4)               default ''::character varying                   not null,
    gross_profit              numeric(18, 4)           default 0                                       not null,
    operating_profit_margin   numeric(18, 4)           default 0                                       not null,
    "pre-tax_income"          numeric(18, 4)           default 0                                       not null,
    net_income                numeric(18, 4)           default 0                                       not null,
    net_asset_value_per_share numeric(18, 4)           default 0                                       not null,
    sales_per_share           numeric(18, 4)           default 0                                       not null,
    earnings_per_share        numeric(18, 4)           default 0                                       not null,
    profit_before_tax         numeric(18, 4)           default 0                                       not null,
    return_on_equity          numeric(18, 4)           default 0                                       not null,
    return_on_assets          numeric(18, 4)           default 0                                       not null,
    created_time              timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    updated_time              timestamp with time zone default ('now'::text)::timestamp with time zone not null
);

comment on column public.financial_statement.year is '年度';
comment on column public.financial_statement.quarter is '季度 A:全年度 Q1~Q4:第一季~第四季';
comment on column public.financial_statement.gross_profit is '營業毛利率';
comment on column public.financial_statement.operating_profit_margin is '營業利益率';
comment on column public.financial_statement."pre-tax_income" is '稅前淨利率';
comment on column public.financial_statement.net_income is '稅後淨利率';
comment on column public.financial_statement.net_asset_value_per_share is '每股淨值';
comment on column public.financial_statement.sales_per_share is '每股營收';
comment on column public.financial_statement.earnings_per_share is '每股稅後淨利';
comment on column public.financial_statement.profit_before_tax is '每股稅前淨利';
comment on column public.financial_statement.return_on_equity is '股東權益報酬率';
comment on column public.financial_statement.return_on_assets is '資產報酬率';

create unique index "financial_statement-security_code-year-quarter-uidx"
    on public.financial_statement (security_code asc, year desc, quarter desc);

create index "financial_statement-year-quarter-idx"
    on public.financial_statement (year, quarter) include (security_code, earnings_per_share);

