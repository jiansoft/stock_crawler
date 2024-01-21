create table public.stock_industry
(
    stock_industry_id integer     default 0                     not null
        primary key,
    name              varchar(24) default ''::character varying not null
);

comment on column public.stock_industry.stock_industry_id is '股票的產業分類編號';
comment on column public.stock_industry.name is '分類名稱';


insert into stock_industry (stock_industry_id, name)
values (1, '水泥工業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (2, '食品工業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (3, '塑膠工業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (4, '紡織纖維')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (5, '電機機械')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (6, '電器電纜')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (8, '玻璃陶瓷')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (9, '造紙工業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (10, '鋼鐵工業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (11, '橡膠工業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (12, '汽車工業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (13, '電子工業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (14, '建材營造業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (15, '航運業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (16, '觀光餐旅')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (17, '金融保險業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (18, '貿易百貨業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (19, '綜合')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (20, '其他業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (21, '化學工業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (22, '生技醫療業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (23, '油電燃氣業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (24, '半導體業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (25, '電腦及週邊設備業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (26, '光電業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (27, '通信網路業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (28, '電子零組件業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (29, '電子通路業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (30, '資訊服務業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (31, '其他電子業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (32, '文化創意業')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (33, '農業科技')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (34, '電子商務')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (35, '綠能環保')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (36, '數位雲端')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (37, '運動休閒')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (38, '居家生活')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (39, '存託憑證')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (99, '未分類')
    on conflict (stock_industry_id) DO UPDATE SET name = excluded.name;
