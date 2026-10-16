//! 訊號績效、大盤基準與個股訊號歷史 SQL 的查詢列。

use chrono::NaiveDate;
use rust_decimal::Decimal;

#[cfg(doc)]
use super::HORIZONS;
#[cfg(doc)]
use super::sql::{BENCHMARK_SQL, PERFORMANCE_SQL, STOCK_SIGNALS_SQL};

/// 單一天期的統計值。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct HorizonValues {
    pub(super) samples: i64,
    pub(super) avg: Option<f64>,
    pub(super) median: Option<f64>,
    pub(super) win: Option<f64>,
    pub(super) excess: Option<f64>,
    pub(super) beat: Option<f64>,
}

/// [`PERFORMANCE_SQL`] 的一列。
#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub(in crate::interfaces::web::data_api::handlers) struct PerformanceRow {
    pub(super) indicator: String,
    pub(super) kind: String,
    /// 年度；整段期間的統計（或沒有依年度拆分的查詢）為 `None`。
    #[sqlx(default)]
    pub(super) year: Option<i32>,
    pub(in crate::interfaces::web::data_api::handlers) occurrences: i64,
    pub(super) n5: i64,
    pub(super) avg5: Option<f64>,
    pub(super) med5: Option<f64>,
    pub(super) win5: Option<f64>,
    pub(super) excess5: Option<f64>,
    pub(super) beat5: Option<f64>,
    pub(super) n20: i64,
    pub(super) avg20: Option<f64>,
    pub(super) med20: Option<f64>,
    pub(super) win20: Option<f64>,
    pub(super) excess20: Option<f64>,
    pub(super) beat20: Option<f64>,
    pub(super) n60: i64,
    pub(super) avg60: Option<f64>,
    pub(super) med60: Option<f64>,
    pub(super) win60: Option<f64>,
    pub(super) excess60: Option<f64>,
    pub(super) beat60: Option<f64>,
    pub(super) n120: i64,
    pub(super) avg120: Option<f64>,
    pub(super) med120: Option<f64>,
    pub(super) win120: Option<f64>,
    pub(super) excess120: Option<f64>,
    pub(super) beat120: Option<f64>,
    pub(super) n240: i64,
    pub(super) avg240: Option<f64>,
    pub(super) med240: Option<f64>,
    pub(super) win240: Option<f64>,
    pub(super) excess240: Option<f64>,
    pub(super) beat240: Option<f64>,
}

impl PerformanceRow {
    /// 依 [`HORIZONS`] 順序的各天期統計。
    pub(super) fn horizons(&self) -> [HorizonValues; 5] {
        [
            HorizonValues {
                samples: self.n5,
                avg: self.avg5,
                median: self.med5,
                win: self.win5,
                excess: self.excess5,
                beat: self.beat5,
            },
            HorizonValues {
                samples: self.n20,
                avg: self.avg20,
                median: self.med20,
                win: self.win20,
                excess: self.excess20,
                beat: self.beat20,
            },
            HorizonValues {
                samples: self.n60,
                avg: self.avg60,
                median: self.med60,
                win: self.win60,
                excess: self.excess60,
                beat: self.beat60,
            },
            HorizonValues {
                samples: self.n120,
                avg: self.avg120,
                median: self.med120,
                win: self.win120,
                excess: self.excess120,
                beat: self.beat120,
            },
            HorizonValues {
                samples: self.n240,
                avg: self.avg240,
                median: self.med240,
                win: self.win240,
                excess: self.excess240,
                beat: self.beat240,
            },
        ]
    }
}

/// [`BENCHMARK_SQL`] 的結果。
#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub(in crate::interfaces::web::data_api::handlers) struct BenchmarkRow {
    /// 年度；整段期間（或沒有依年度拆分的查詢）為 `None`。
    #[sqlx(default)]
    pub(super) year: Option<i32>,
    pub(super) n5: i64,
    pub(super) avg5: Option<f64>,
    pub(super) win5: Option<f64>,
    pub(super) n20: i64,
    pub(super) avg20: Option<f64>,
    pub(super) win20: Option<f64>,
    pub(super) n60: i64,
    pub(super) avg60: Option<f64>,
    pub(super) win60: Option<f64>,
    pub(super) n120: i64,
    pub(super) avg120: Option<f64>,
    pub(super) win120: Option<f64>,
    pub(super) n240: i64,
    pub(super) avg240: Option<f64>,
    pub(super) win240: Option<f64>,
}

/// [`STOCK_SIGNALS_SQL`] 的一列。
#[derive(Debug, Clone, sqlx::FromRow)]
pub(super) struct StockSignalRow {
    pub(super) date: NaiveDate,
    pub(super) indicator: String,
    pub(super) kind: String,
    pub(super) description: String,
    pub(super) closing_price: Option<Decimal>,
    pub(super) change_range: Option<Decimal>,
    pub(super) return_5d: Option<Decimal>,
    pub(super) return_20d: Option<Decimal>,
    pub(super) return_60d: Option<Decimal>,
    pub(super) return_120d: Option<Decimal>,
    pub(super) return_240d: Option<Decimal>,
    pub(super) total: i64,
}
