-- DROP TABLE IF EXISTS "public"."market_category";
create table public.market_category
(
    market_category_id integer     default 0                     not null
        primary key,
    exchange           varchar(32) default ''::character varying not null,
    name               varchar(32) default ''::character varying not null
);

comment on column public.market_category.market_category_id is '市場分類編號';
comment on column public.market_category.exchange is '市場別 TAI:上市 TWO:上櫃 TWE:興櫃';
comment on column public.market_category.name is '市場名稱';

insert into public.market_category (exchange, market_category_id, name)
values ('TAI', 2, '上市'),
       ('TWO', 4, '上櫃'),
       ('TWE', 5, '興櫃');