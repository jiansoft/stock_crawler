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


-- 只有每天新增（約 1～1.5 千列），預設 insert 門檻 20% 要數月才 vacuum 一次，新頁面沒有 visibility map；
-- analyze 門檻 10% 也讓統計值逐漸過時（2026-11-04 M43 盤點）。改成每新增／變動 2 萬列就 vacuum／analyze。
-- 正式庫 2026-11-05 已套用（M44）。
alter table public.yield_rank set (
    autovacuum_vacuum_insert_threshold = 20000,
    autovacuum_vacuum_insert_scale_factor = 0,
    autovacuum_analyze_threshold = 20000,
    autovacuum_analyze_scale_factor = 0
);
