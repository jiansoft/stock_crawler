-- 股利發放明細
-- DROP TABLE IF EXISTS dividend;
CREATE TABLE dividend
(
    serial                         BIGSERIAL      NOT NULL,
    security_code                  VARCHAR(24)    NOT NULL  DEFAULT ''::CHARACTER VARYING,
    year                           INT4           NOT NULL  DEFAULT 0,
    year_of_dividend               INT4           NOT NULL  DEFAULT 0,
    quarter                        VARCHAR(4)     NOT NULL  DEFAULT ''::CHARACTER VARYING,
    cash_dividend                  NUMERIC(18, 4) NOT NULL  DEFAULT 0,
    capital_reserve_cash_dividend  NUMERIC(18, 4) NOT NULL  DEFAULT 0,
    earnings_cash_dividend         NUMERIC(18, 4) NOT NULL  DEFAULT 0,
    stock_dividend                 NUMERIC(18, 4) NOT NULL  DEFAULT 0,
    capital_reserve_stock_dividend NUMERIC(18, 4) NOT NULL  DEFAULT 0,
    earnings_stock_dividend        NUMERIC(18, 4) NOT NULL  DEFAULT 0,
    sum                            NUMERIC(18, 4) NOT NULL  DEFAULT 0,
    payout_ratio_cash              NUMERIC(18, 4) NOT NULL  DEFAULT 0,
    payout_ratio_stock             NUMERIC(18, 4) NOT NULL  DEFAULT 0,
    payout_ratio                   NUMERIC(18, 4) NOT NULL  DEFAULT 0,
    "ex-dividend_date1"            VARCHAR(10)    NOT NULL  DEFAULT ''::CHARACTER VARYING,
    "ex-dividend_date2"            VARCHAR(10)    NOT NULL  DEFAULT ''::CHARACTER VARYING,
    payable_date1                  VARCHAR(10)    NOT NULL  DEFAULT ''::CHARACTER VARYING,
    payable_date2                  VARCHAR(10)    NOT NULL  DEFAULT ''::CHARACTER VARYING,
    created_time                   TIMESTAMP WITH TIME ZONE DEFAULT ('now'::TEXT)::TIMESTAMP WITH TIME ZONE NOT NULL,
    updated_time                   TIMESTAMP WITH TIME ZONE DEFAULT ('now'::TEXT)::TIMESTAMP WITH TIME ZONE NOT NULL,
    CONSTRAINT "dividend_pkey" PRIMARY KEY (security_code, year, quarter)
);

COMMENT ON COLUMN dividend.year IS '股利發放年度';
COMMENT ON COLUMN dividend.year_of_dividend IS '股利所屬年度';
COMMENT ON COLUMN dividend.cash_dividend IS '現金股利';
COMMENT ON COLUMN dividend.capital_reserve_cash_dividend IS '公積現金股利';
COMMENT ON COLUMN dividend.earnings_cash_dividend IS '盈餘現金股利';
COMMENT ON COLUMN dividend.stock_dividend IS '股票股利';
COMMENT ON COLUMN dividend.capital_reserve_stock_dividend IS '公積股票股利';
COMMENT ON COLUMN dividend.earnings_stock_dividend IS '盈餘股票股利';
COMMENT ON COLUMN dividend.quarter IS '季度 空字串:全年度 Q1~Q4:第一季~第四季 H1~H2︰上半季~下半季';
COMMENT ON COLUMN dividend."ex-dividend_date1" IS '除息日';
COMMENT ON COLUMN dividend."ex-dividend_date2" IS '除權日';
COMMENT ON COLUMN dividend.payable_date1 IS '現金股利發放日';
COMMENT ON COLUMN dividend.payable_date2 IS '股票股利發放日';
COMMENT ON COLUMN dividend.sum IS '合計';
COMMENT ON COLUMN dividend.payout_ratio_cash IS '盈餘分配率_現金(%)';
COMMENT ON COLUMN dividend.payout_ratio_stock IS '盈餘分配率_股要(%)';
COMMENT ON COLUMN dividend.payout_ratio IS '盈餘分配率(%)';


CREATE INDEX "dividend-serial-idx" ON dividend USING btree ("serial");
CREATE INDEX "dividend-year-dividend_date-idx" ON dividend USING btree ("year", "ex-dividend_date1", "ex-dividend_date2");
CREATE INDEX "dividend-year-payable_date-idx" ON dividend USING btree ("year", "payable_date1", "payable_date2");
