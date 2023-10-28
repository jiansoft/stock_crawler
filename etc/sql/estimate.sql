create table public.estimate
(
    serial             bigserial
        primary key,
    security_code      varchar(24)              default ''::character varying                   not null,
    date               date                     default CURRENT_DATE                            not null,
    percentage         numeric(18, 4)           default 0                                       not null,
    closing_price      numeric(18, 4)           default 0                                       not null,
    cheap              numeric(18, 4)           default 0                                       not null,
    fair               numeric(18, 4)           default 0                                       not null,
    expensive          numeric(18, 4)           default 0                                       not null,
    price_cheap        numeric(18, 4)           default 0                                       not null,
    price_fair         numeric(18, 4)           default 0                                       not null,
    price_expensive    numeric(18, 4)           default 0                                       not null,
    dividend_cheap     numeric(18, 4)           default 0                                       not null,
    dividend_fair      numeric(18, 4)           default 0                                       not null,
    dividend_expensive numeric(18, 4)           default 0                                       not null,
    year_count         integer                  default 0                                       not null,
    create_time        timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    update_time        timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    eps_cheap          numeric(18, 4)           default 0                                       not null,
    eps_fair           numeric(18, 4)           default 0                                       not null,
    eps_expensive      numeric(18, 4)           default 0                                       not null,
    pbr_cheap          numeric(18, 4)           default 0                                       not null,
    pbr_fair           numeric(18, 4)           default 0                                       not null,
    pbr_expensive      numeric(18, 4)           default 0                                       not null,
    per_cheap          numeric(18, 4)           default 0                                       not null,
    per_fair           numeric(18, 4)           default 0                                       not null,
    per_expensive      numeric(18, 4)           default 0                                       not null
);

comment on column public.estimate.date is '資料屬於那一天';
comment on column public.estimate.percentage is '收盤價與價宜價之價差百分比';
comment on column public.estimate.closing_price is '收盤價';
comment on column public.estimate.cheap is '便宜價';
comment on column public.estimate.fair is '合理價';
comment on column public.estimate.expensive is '昂貴價';
comment on column public.estimate.price_cheap is '歷年股價平均的便宜價';
comment on column public.estimate.price_fair is '歷年股價平均的合理價';
comment on column public.estimate.price_expensive is '歷年股價平均的昂貴價';
comment on column public.estimate.dividend_cheap is '當期股利+歷年股利推估的便宜價 15倍約等於殖利率6.6%';
comment on column public.estimate.dividend_fair is '當期股利+歷年股利推估的合理價 20倍約等於殖利率5%';
comment on column public.estimate.dividend_expensive is '當期股利+歷年股利推估的昂貴價 30倍約等於殖利率3.3%';
comment on column public.estimate.eps_cheap is '近四季EPS * 歷年平均盈餘分配率 15倍';
comment on column public.estimate.eps_fair is '近四季EPS * 歷年平均盈餘分配率 20倍';
comment on column public.estimate.eps_expensive is '近四季EPS * 歷年平均盈餘分配率 30倍';
comment on column public.estimate.pbr_cheap is '歷年20%百分位數股價淨值比的值 * 當日每股淨值';
comment on column public.estimate.pbr_fair is '歷年50%百分位數股價淨值比的值 * 當日每股淨值';
comment on column public.estimate.pbr_expensive is '歷年80%百分位數股價淨值比的值 * 當日每股淨值';
comment on column public.estimate.per_cheap is '歷年10%百分位數本益比比的值 * 歷年EPS平均值';
comment on column public.estimate.per_fair is '歷年50%百分位數本益比比的值 * 歷年EPS平均值';
comment on column public.estimate.per_expensive is '歷年80%百分位數本益比的值 * 歷年EPS平均值';

create unique index "estimate-security_code-date-uidx"
    on public.estimate (security_code, date);

