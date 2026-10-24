-- ETF 相對既有可投資代理基準的含息日報酬追蹤誤差，非實際標的指數追蹤誤差。
CREATE TABLE IF NOT EXISTS etf_tracking_error (
    stock_symbol text NOT NULL,
    benchmark_symbol text NOT NULL,
    calc_date date NOT NULL,
    samples integer NOT NULL CHECK (samples >= 0),
    tracking_error_percent double precision CHECK (tracking_error_percent >= 0),
    tracking_difference_percent double precision,
    PRIMARY KEY (stock_symbol, benchmark_symbol)
);
