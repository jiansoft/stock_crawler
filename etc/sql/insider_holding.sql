create table if not exists public.insider_holding
(
    "month"         date                                                   not null,
    stock_symbol    varchar(24)              default ''::character varying not null,
    title           varchar(64)              default ''::character varying not null,
    name            varchar(128)             default ''::character varying not null,
    shares          bigint                   default 0                     not null,
    pledged         bigint                   default 0                     not null,
    related_pledged bigint                   default 0                     not null,
    created_time    timestamp with time zone default now()                 not null,
    updated_time    timestamp with time zone default now()                 not null,
    primary key ("month", stock_symbol, title, name)
);

comment on table public.insider_holding is '董監事持股與設質月報：上市 t187ap11_L、上櫃 mopsfin_t187ap11_O（開放資料只有最新月份，無法回補）';

comment on column public.insider_holding."month" is '資料月份（該月 1 日）';
comment on column public.insider_holding.stock_symbol is '股票代號';
comment on column public.insider_holding.title is '職稱';
comment on column public.insider_holding.name is '姓名（法人代表列為法人名稱）';
comment on column public.insider_holding.shares is '目前持股（股）';
comment on column public.insider_holding.pledged is '設質股數（股）';
comment on column public.insider_holding.related_pledged is '內部人關係人設質股數（股）';

create index if not exists "insider_holding-stock_symbol-month-idx"
    on public.insider_holding (stock_symbol, "month" desc);
