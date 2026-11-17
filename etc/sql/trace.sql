-- 價格追蹤：每位會員各一份清單（會員系統）。指向 "Member" 的外來鍵由 member_rbac_and_mcp_token.sql 加上
-- （本檔在 CI 載入順序中早於 member.sql）；member_id 必須明確指定。
create table public.trace
(
    member_id    bigint         not null,
    stock_symbol varchar(24)    default ''::character varying not null,
    floor        numeric(18, 4) default 0                     not null,
    ceiling      numeric(18, 4) default 0                     not null,
    primary key (member_id, stock_symbol)
);

create index if not exists idx_trace_stock_symbol on public.trace (stock_symbol);

comment on column public.trace.member_id is '設定這筆追蹤條件的會員';

comment on column public.trace.floor is '低於此價格時發送通知';
comment on column public.trace.ceiling is '高於此價格時發送通知';