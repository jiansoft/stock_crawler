--DROP TABLE IF EXISTS daily_money_history;

-- Creating the table
CREATE TABLE daily_money_history
(
    date       DATE                     DEFAULT CURRENT_DATE      NOT NULL PRIMARY KEY,
    sum        NUMERIC(18, 4)           DEFAULT 0                 NOT NULL,
    eddie      NUMERIC(18, 4)           DEFAULT 0                 NOT NULL,
    unice      NUMERIC(18, 4)           DEFAULT 0                 NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
);

COMMENT ON COLUMN daily_money_history.date IS '資料屬於那一天';

CREATE UNIQUE INDEX "idx-daily_money_history-date" ON daily_money_history (date DESC);
