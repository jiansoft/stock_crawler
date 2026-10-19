-- Member：舊版會員表（PascalCase 欄位；早期 stock_go 建立）。目前 stock_rust、stock_go、stock_api 皆未引用，保留結構以便重建正式庫。
-- 結構由 2026-10-19 正式庫 pg_dump --schema-only 導出。

CREATE TABLE public."Member" (
    "Id" bigint NOT NULL,
    "Account" character varying(24) DEFAULT ''::character varying NOT NULL,
    "Password" character varying(24) DEFAULT ''::character varying NOT NULL,
    "Name" character varying(255) DEFAULT ''::character varying NOT NULL,
    "CreateTime" timestamp with time zone DEFAULT ('now'::text)::timestamp with time zone NOT NULL
);

CREATE SEQUENCE public."Member_Id_seq"
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE public."Member_Id_seq" OWNED BY public."Member"."Id";

ALTER TABLE ONLY public."Member" ALTER COLUMN "Id" SET DEFAULT nextval('public."Member_Id_seq"'::regclass);

ALTER TABLE ONLY public."Member"
    ADD CONSTRAINT "Member_pkey" PRIMARY KEY ("Id");

CREATE INDEX "Member_Account" ON public."Member" USING btree ("Account");
