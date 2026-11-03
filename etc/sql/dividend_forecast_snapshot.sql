-- 股利預估快照：每月 1 日 08:15 記錄各會員 `/portfolio/dividend-forecast` 未來 12 個月的逐筆預估，
-- 事後與已領股利明細比對預估誤差（T24）。同一快照日重跑時整批替換。
create table public.dividend_forecast_snapshot
(
    snapshot_date  date                     not null,
    member_id      bigint                   not null,
    stock_symbol   varchar(24)              not null,
    ex_date        date                     not null,
    pay_date       date                     not null,
    shares         bigint                   not null,
    cash_per_share numeric(18, 4)           not null,
    amount         numeric(18, 4)           not null,
    basis          varchar(24)              not null,
    entitled       boolean     default false not null,
    created_time   timestamp with time zone default now() not null,
    primary key (snapshot_date, member_id, stock_symbol, ex_date)
);

comment on table public.dividend_forecast_snapshot is '股利預估快照（每月 1 日，未來 12 個月）';
comment on column public.dividend_forecast_snapshot.snapshot_date is '快照日';
comment on column public.dividend_forecast_snapshot.member_id is '會員編號';
comment on column public.dividend_forecast_snapshot.stock_symbol is '股票代號';
comment on column public.dividend_forecast_snapshot.ex_date is '除息日（推估者為推估日期）';
comment on column public.dividend_forecast_snapshot.pay_date is '發放日（推估者為推估日期）';
comment on column public.dividend_forecast_snapshot.shares is '預估時的股數（已除息者為除息日持股）';
comment on column public.dividend_forecast_snapshot.cash_per_share is '每股現金股利';
comment on column public.dividend_forecast_snapshot.amount is '預估入帳金額';
comment on column public.dividend_forecast_snapshot.basis is '預估依據：announced、amount_announced、estimated、latest_amount、cadence';
comment on column public.dividend_forecast_snapshot.entitled is '已除息，股數與金額取已領股利明細';

-- 比對範例：某次快照中發放日已過的預估 vs 已領股利明細（依會員、代號、除息日）。
-- SELECT f.snapshot_date, f.member_id, f.stock_symbol, f.ex_date, f.pay_date, f.basis,
--        f.amount AS forecast, COALESCE(sum(m.cash), 0) AS actual,
--        COALESCE(sum(m.cash), 0) - f.amount AS diff
-- FROM dividend_forecast_snapshot f
-- LEFT JOIN dividend d ON d.security_code = f.stock_symbol AND d.quarter <> ''
--      AND d."ex-dividend_date1" = to_char(f.ex_date, 'YYYY-MM-DD')
-- LEFT JOIN dividend_record_detail_more m ON m.dividend_serial = d.serial
-- LEFT JOIN stock_ownership_details o ON o.serial = m.stock_ownership_details_serial AND o.member_id = f.member_id
-- WHERE f.snapshot_date = '2026-12-01' AND f.pay_date < current_date
-- GROUP BY f.snapshot_date, f.member_id, f.stock_symbol, f.ex_date, f.pay_date, f.basis, f.amount
-- ORDER BY f.pay_date, f.stock_symbol;
