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

-- 比對範例：某次快照中發放日已過的預估 vs 同一會員的已領股利明細（依代號、除息日）。
-- 已領金額必須在子查詢內以 `o.member_id = f.member_id` 過濾後再加總；若把持股明細放在外層 LEFT JOIN，
-- 過濾條件只會讓 o 變成 NULL，其他會員的 m.cash 仍會被算進來。
-- 正式的誤差計算以 `/portfolio/dividend-forecast/accuracy` 為準：它依（代號, 發放月份）比對該會員的
-- 實際入帳（`domain::portfolio::forecast_accuracy::compare`），推估日期與實際日期不同時也對得上，
-- 結果可能與此依除息日比對的範例不同。
-- SELECT f.snapshot_date, f.member_id, f.stock_symbol, f.ex_date, f.pay_date, f.basis,
--        f.amount AS forecast, COALESCE(a.actual, 0) AS actual,
--        COALESCE(a.actual, 0) - f.amount AS diff
-- FROM dividend_forecast_snapshot f
-- LEFT JOIN LATERAL (
--     SELECT sum(m.cash) AS actual
--     FROM dividend d
--     JOIN dividend_record_detail_more m ON m.dividend_serial = d.serial
--     JOIN stock_ownership_details o ON o.serial = m.stock_ownership_details_serial
--     WHERE d.security_code = f.stock_symbol AND d.quarter <> ''
--       AND d."ex-dividend_date1" = to_char(f.ex_date, 'YYYY-MM-DD')
--       AND o.member_id = f.member_id
-- ) a ON true
-- WHERE f.snapshot_date = '2026-12-01' AND f.pay_date < current_date
-- ORDER BY f.member_id, f.pay_date, f.stock_symbol;
