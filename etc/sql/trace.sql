create table public.trace
(
    stock_symbol varchar(24)    default ''::character varying not null
        primary key,
    floor        numeric(18, 4) default 0                     not null,
    ceiling      numeric(18, 4) default 0                     not null
);

comment on column public.trace.floor is '低於此價格時發送通知';
comment on column public.trace.ceiling is '高於此價格時發送通知';