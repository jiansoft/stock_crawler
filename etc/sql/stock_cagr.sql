create table if not exists public.stock_cagr
(
    date              date                     default CURRENT_DATE          not null,
    stock_symbol      varchar(24)              default ''::character varying not null,
    period            varchar(8)               default ''::character varying not null,
    base_date         date,
    base_price        numeric(18, 4),
    end_price         numeric(18, 4),
    years             numeric(10, 6),
    -- 口徑 A：純價格
    price_end_shares  numeric(28, 8),
    price_end_value   numeric(18, 4),
    price_return_pct  numeric(18, 4),
    price_cagr_pct    numeric(18, 4),
    -- 口徑 B：含息不再投入（主指標）
    total_shares      numeric(28, 8),
    total_cash        numeric(18, 4),
    total_end_value   numeric(18, 4),
    total_return_pct  numeric(18, 4),
    total_cagr_pct    numeric(18, 4),
    -- 口徑 C：含息再投入
    reinv_shares      numeric(28, 8),
    reinv_cash        numeric(18, 4),
    reinv_end_value   numeric(18, 4),
    reinv_return_pct  numeric(18, 4),
    reinv_cagr_pct    numeric(18, 4),
    -- 品質旗標
    first_quote_date  date,
    shortfall_days    integer,
    data_complete     boolean                  default false                 not null,
    has_anomaly       boolean                  default false                 not null,
    dividend_events   integer                  default 0                     not null,
    created_time      timestamp with time zone default now()                 not null,
    updated_time      timestamp with time zone default now()                 not null,
    primary key (date, stock_symbol, period)
);

comment on table public.stock_cagr is '每日 CAGR：固定投入金額於各統計期間、各報酬口徑的模擬結果';

comment on column public.stock_cagr.date is '計算基準日（期末交易日）';
comment on column public.stock_cagr.stock_symbol is '股票代號';
comment on column public.stock_cagr.period is '統計期間代碼：M3/M6/Y1/Y1H/Y2/Y3/Y5/Y10';
comment on column public.stock_cagr.base_date is '實際採用的期初交易日（名目期初日無報價時可順延，見 BASE_DATE_GRACE_DAYS）';
comment on column public.stock_cagr.base_price is '期初收盤價';
comment on column public.stock_cagr.end_price is '期末收盤價';
comment on column public.stock_cagr.years is '實際年數 =（期末交易日 − 期初交易日）/ 365';

comment on column public.stock_cagr.price_end_shares is '口徑 A（純價格）：期末持有股數';
comment on column public.stock_cagr.price_end_value is '口徑 A（純價格）：期末總價值（元）';
comment on column public.stock_cagr.price_return_pct is '口徑 A（純價格）：區間總報酬率（%）';
comment on column public.stock_cagr.price_cagr_pct is '口徑 A（純價格）：年化報酬率（%）';

comment on column public.stock_cagr.total_shares is '口徑 B（含息不再投入）：期末持有股數（含配股）';
comment on column public.stock_cagr.total_cash is '口徑 B（含息不再投入）：累積現金股利（元）';
comment on column public.stock_cagr.total_end_value is '口徑 B（含息不再投入）：期末總價值（元）';
comment on column public.stock_cagr.total_return_pct is '口徑 B（含息不再投入）：區間總報酬率（%）';
comment on column public.stock_cagr.total_cagr_pct is '口徑 B（含息不再投入）：年化報酬率（%），為主指標';

comment on column public.stock_cagr.reinv_shares is '口徑 C（含息再投入）：期末持有股數（含配股與現金股利買回）';
comment on column public.stock_cagr.reinv_cash is '口徑 C（含息再投入）：未能買回的剩餘現金（元），通常為零';
comment on column public.stock_cagr.reinv_end_value is '口徑 C（含息再投入）：期末總價值（元）';
comment on column public.stock_cagr.reinv_return_pct is '口徑 C（含息再投入）：區間總報酬率（%）';
comment on column public.stock_cagr.reinv_cagr_pct is '口徑 C（含息再投入）：年化報酬率（%）';

comment on column public.stock_cagr.first_quote_date is '該股在 DailyQuotes 的最早報價日，用於判定新上市／資料未涵蓋';
comment on column public.stock_cagr.shortfall_days is 'base_date 較名目期初目標日晚幾天；0 表完全對齊';
comment on column public.stock_cagr.data_complete is '期初資料是否齊全；false 時所有數值欄位皆為 NULL（絕不以 0 代替）';
comment on column public.stock_cagr.has_anomaly is '期間內是否偵測到無對應除權息的價格跳動（疑似減資或分割）';
comment on column public.stock_cagr.dividend_events is '期間內實際採計的除權息次數';

-- 排行榜索引（每個口徑各一條）。
-- 查詢型態固定為「指定 date + period，依某口徑年化報酬率由高至低取前 N 筆」，
-- 因此以 (date desc, period, <口徑>_cagr_pct desc) 排列即可直接依序掃描；
-- INCLUDE 讓排行榜常用的代號與區間總報酬免於回表。
-- partial 條件 data_complete = true 與查詢條件一致，可大幅縮小索引體積
-- （長期間如 Y10 約有四成列為資料不足）。
create index if not exists "stock_cagr-rank-total-idx"
    on public.stock_cagr (date desc, period, total_cagr_pct desc)
    include (stock_symbol, total_return_pct)
    where data_complete = true;

create index if not exists "stock_cagr-rank-price-idx"
    on public.stock_cagr (date desc, period, price_cagr_pct desc)
    include (stock_symbol, price_return_pct)
    where data_complete = true;

create index if not exists "stock_cagr-rank-reinv-idx"
    on public.stock_cagr (date desc, period, reinv_cagr_pct desc)
    include (stock_symbol, reinv_return_pct)
    where data_complete = true;

-- 個股歷史索引：查詢單一個股某期間的時間序列（含資料不足者），故不加 partial 條件。
create index if not exists "stock_cagr-symbol-period-date-idx"
    on public.stock_cagr (stock_symbol, period, date desc);
