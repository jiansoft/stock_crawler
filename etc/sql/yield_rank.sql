create table if not exists public.yield_rank
(
    serial              bigserial
        primary key,
    date                date                     default CURRENT_DATE                            not null,
    security_code       varchar(24)              default ''::character varying                   not null,
    daily_quotes_serial bigint                   default 0                                       not null,
    dividend_serial     bigint                   default 0                                       not null,
    yield               numeric(18, 4)           default 0                                       not null,
    created_time        timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    updated_time        timestamp with time zone default ('now'::text)::timestamp with time zone not null
);

comment on column public.yield_rank.date is '資料屬於那一天';
comment on column public.yield_rank.daily_quotes_serial is '當日收盤價格對應 DailyQuotes.Serial';
comment on column public.yield_rank.dividend_serial is '該股票合記配發的現金與股票股利對應 Dividend.Serial';
comment on column public.yield_rank.yield is '當日的殖利率';

create unique index if not exists "yield_rank-date-security_code-idx"
    on public.yield_rank (date, security_code) include (daily_quotes_serial, dividend_serial);

create index if not exists "yield_rank-security_code-idx"
    on public.yield_rank (security_code);

-- screen_stocks 需要依股票取最新一筆殖利率；單欄 security_code 索引仍須為每檔
-- 股票掃過大量日期。複合索引直接以 date DESC 取第一筆，INCLUDE 避免回表取 yield。
create index if not exists "yield_rank-security_code-date-desc-idx"
    on public.yield_rank (security_code, date desc) include (yield);

