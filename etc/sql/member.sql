-- Member：會員表（PascalCase 欄位；早期 stock_go 建立）。stock_go 以 "Member" 取會員姓名
-- （dividend_record_repository.go 的 LEFT JOIN "Member"），會員系統的認證欄位、家庭與群組
-- 由 member_rbac_and_mcp_token.sql 以 ALTER TABLE 擴充（保留表名與既有欄位，零破壞）。
-- 本檔是擴充前的原始結構，由 2026-10-19 正式庫 pg_dump --schema-only 導出；載入順序須在結構檔之前。

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
