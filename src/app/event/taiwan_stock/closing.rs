use crate::{
    app::backfill,
    app::calculation,
    core::{alert, util::datetime::Weekend},
    domain::quote::{
        entity::MarketTradedCount,
        forward_return::{self, DerivedCounts, DerivedDataQuery, ForwardReturnRepository},
        repository::QuoteRepository,
    },
    infra::cache::{TTL, TtlCacheInner},
    infra::crawler::{self, twse},
    infra::database::repository::{quote::PgQuoteRepository, yield_rank::PgYieldRankRepository},
};
use anyhow::Result;
use chrono::{DateTime, Datelike, Local, NaiveDate};
use scopeguard::defer;

/// 台股收盤事件發生時要進行的事情
pub async fn execute() -> Result<()> {
    tracing::info!("台股收盤事件開始");
    defer! {
       tracing::info!("台股收盤事件結束");
    }

    let now = Local::now();
    let current_date: NaiveDate = now.date_naive();
    if is_market_closed(now).await {
        tracing::info!("{current_date} 休市，略過收盤匯總");
        return Ok(());
    }

    let aggregate = aggregate(current_date);
    let index = backfill::taiwan_stock_index::execute();
    let (res_aggregation, res_index) = tokio::join!(aggregate, index);

    if let Err(why) = res_index {
        tracing::error!("Failed to taiwan_stock_index::execute() because {:#?}", why);
    }

    if let Err(why) = res_aggregation {
        tracing::error!("Failed to closing::aggregate() because {:#?}", why);
    }

    // 停止 trace 事件所使用的即時報價背景任務
    crate::app::event::trace::price_tasks::stop_price_tasks().await;

    crawler::flush_site_latency_stats();

    Ok(())
}

/// 收盤資料完整性檢查回看的天數，涵蓋春節等長假，確保至少有前幾個交易日可比較。
const COVERAGE_LOOKBACK_DAYS: i64 = 14;

/// 單一市場當天有成交的檔數低於近期交易日最大值的這個百分比，就視為資料不完整。
///
/// 正常交易日有成交的檔數很穩定（上市約 1,000、上櫃約 880），整個市場沒抓到時會掉到 0，
/// 遠低於門檻；冷門股沒成交造成的波動只有數十檔。
const COVERAGE_MIN_PERCENT: i64 = 50;

/// 收盤資料不完整的市場。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shortfall {
    /// 市場名稱。
    pub(crate) market: &'static str,
    /// 當天有成交的檔數。
    pub(crate) traded: i64,
    /// 近期交易日有成交檔數的最大值。
    pub(crate) baseline: i64,
}

/// 排程入口：檢查今天的收盤資料是否完整，不完整就重跑收盤匯總並告警。
///
/// 15:00 的收盤匯總可能因來源尚未公布或暫時失敗而沒有寫入，或只有一個市場有資料；
/// 舊版沒有任何重試，該日就一直缺著，「缺漏補齊」還會把缺的市場補成整天零量列
/// （2018～2026 年共找到 15 個這樣的交易日）。這裡在盤後再檢查：上市或上櫃有成交的檔數
/// 低於近期交易日的 [`COVERAGE_MIN_PERCENT`]% 就重跑 [`aggregate`]，並把結果告警出來。
///
/// 行情檢查之後再以 [`ensure_derived`] 檢查技術訊號與未來報酬。
///
/// # Errors
///
/// 查詢有成交檔數或衍生資料筆數失敗時回傳錯誤；重跑失敗只告警，不回傳錯誤。
pub async fn ensure_complete() -> Result<()> {
    let now = Local::now();
    let date = now.date_naive();
    if is_market_closed(now).await {
        return Ok(());
    }

    ensure_quotes(date).await?;
    ensure_derived(date).await
}

/// 上市或上櫃有成交的檔數偏少就重跑收盤匯總並告警。
async fn ensure_quotes(date: NaiveDate) -> Result<()> {
    let repo = PgQuoteRepository::new();
    let from = date - chrono::Duration::days(COVERAGE_LOOKBACK_DAYS);
    let before = shortfalls(date, &repo.fetch_market_traded_counts(from, date).await?);
    if before.is_empty() {
        tracing::info!("{date} 收盤資料完整");
        return Ok(());
    }

    tracing::warn!("{date} 收盤資料不完整，重跑收盤匯總: {before:?}");
    let rerun = aggregate(date).await;
    if let Err(why) = &rerun {
        tracing::error!("{date} 重跑收盤匯總失敗: {why:#}");
    }
    let after = shortfalls(date, &repo.fetch_market_traded_counts(from, date).await?);

    let title = if after.is_empty() {
        "收盤資料已重抓補齊"
    } else {
        "收盤資料仍不完整"
    };
    let error = rerun.err().map(|why| format!("{why:#}"));
    alert::send_alert(title, &report(date, &before, error.as_deref(), &after)).await;
    Ok(())
}

/// 收盤匯總最後兩步產生的衍生資料。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Derived {
    /// 當天的技術訊號（`daily_technical_signal`）。
    Signals,
    /// 5 個交易日前基準日的 5 日報酬（`daily_forward_return`）。
    ForwardReturns,
}

impl Derived {
    fn label(self) -> &'static str {
        match self {
            Self::Signals => "技術訊號",
            Self::ForwardReturns => "未來報酬",
        }
    }
}

/// 行情完整後檢查衍生資料：當天沒有技術訊號，或 5 個交易日前的基準日沒有 5 日報酬
/// （收盤匯總在這兩步之前中斷、或這兩步失敗），就只重算缺的那一步並告警。
///
/// 當天不是交易日（沒有日線）或交易日不足 5 天時不檢查。
async fn ensure_derived(date: NaiveDate) -> Result<()> {
    let report = check_derived(&PgQuoteRepository::new(), date, |derived| async move {
        match derived {
            Derived::Signals => calculation::technical_signal::calculate_technical_signals(date)
                .await
                .map(|_| ()),
            Derived::ForwardReturns => calculation::forward_return::calculate_forward_returns(date)
                .await
                .map(|_| ()),
        }
    })
    .await?;
    if let Some((title, body)) = report {
        alert::send_alert(title, &body).await;
    }
    Ok(())
}

/// [`ensure_derived`] 的本體：檢查、以 `recompute` 重算缺的項目，回傳要發的告警（標題、內容）；
/// 不需要告警時回 `None`。倉儲與重算可替換以便測試。
pub(crate) async fn check_derived<R, F, Fut>(
    repo: &R,
    date: NaiveDate,
    recompute: F,
) -> Result<Option<(&'static str, String)>>
where
    R: ForwardReturnRepository + DerivedDataQuery,
    F: Fn(Derived) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let dates = repo.fetch_trading_dates(date, 6).await?;
    let Some(base) = forward_return::base_date(date, 5, &dates) else {
        return Ok(None);
    };
    let before = missing_derived(repo.count_derived(date, base).await?);
    if before.is_empty() {
        tracing::info!("{date} 技術訊號與未來報酬完整");
        return Ok(None);
    }

    tracing::warn!("{date} 衍生資料缺漏，重算: {before:?}");
    let mut errors = Vec::new();
    for derived in &before {
        if let Err(why) = recompute(*derived).await {
            tracing::error!("{date} 重算{}失敗: {why:#}", derived.label());
            errors.push(format!("{}：{why:#}", derived.label()));
        }
    }
    let after = missing_derived(repo.count_derived(date, base).await?);
    let title = if after.is_empty() {
        "技術訊號／未來報酬已補算"
    } else {
        "技術訊號／未來報酬仍缺漏"
    };
    Ok(Some((
        title,
        derived_report(date, &before, &errors, &after),
    )))
}

/// 筆數為 0 的衍生資料。
pub(crate) fn missing_derived(counts: DerivedCounts) -> Vec<Derived> {
    let mut missing = Vec::new();
    if counts.signals == 0 {
        missing.push(Derived::Signals);
    }
    if counts.forward_returns == 0 {
        missing.push(Derived::ForwardReturns);
    }
    missing
}

/// 衍生資料告警內容：缺哪些、重算失敗的原因、重算後的狀況。
fn derived_report(
    date: NaiveDate,
    before: &[Derived],
    errors: &[String],
    after: &[Derived],
) -> String {
    let names = |items: &[Derived]| {
        items
            .iter()
            .map(|item| item.label())
            .collect::<Vec<_>>()
            .join("、")
    };
    let mut lines = vec![format!("{date} 檢查時缺：{}", names(before))];
    if errors.is_empty() {
        lines.push("已重算".to_owned());
    } else {
        lines.push(format!("重算失敗：{}", errors.join("；")));
    }
    lines.push(if after.is_empty() {
        "重算後都已寫入".to_owned()
    } else {
        format!(
            "重算後仍缺：{}；請查看收盤匯總日誌，必要時手動回補",
            names(after)
        )
    });
    lines.join("\n")
}

/// 比較當天與近期交易日各市場有成交的檔數，列出不完整的市場。
pub(crate) fn shortfalls(date: NaiveDate, counts: &[MarketTradedCount]) -> Vec<Shortfall> {
    [(2, "上市"), (4, "上櫃")]
        .into_iter()
        .filter_map(|(market_id, market)| {
            let in_market = counts.iter().filter(|count| count.market_id == market_id);
            let traded = in_market
                .clone()
                .filter(|count| count.date == date)
                .map(|count| count.traded)
                .sum::<i64>();
            let baseline = in_market
                .filter(|count| count.date < date)
                .map(|count| count.traded)
                .max()
                .unwrap_or(0);
            let short = traded == 0 || traded * 100 < baseline * COVERAGE_MIN_PERCENT;
            short.then_some(Shortfall {
                market,
                traded,
                baseline,
            })
        })
        .collect()
}

/// 告警內容：檢查時缺哪些市場、重跑是否成功、重跑後的狀況。
fn report(
    date: NaiveDate,
    before: &[Shortfall],
    rerun_error: Option<&str>,
    after: &[Shortfall],
) -> String {
    let describe = |shortfalls: &[Shortfall]| {
        shortfalls
            .iter()
            .map(|shortfall| {
                format!(
                    "{} {} 檔有成交（近期 {} 檔）",
                    shortfall.market, shortfall.traded, shortfall.baseline
                )
            })
            .collect::<Vec<_>>()
            .join("、")
    };
    let mut lines = vec![format!("{date} 檢查時：{}", describe(before))];
    lines.push(match rerun_error {
        Some(why) => format!("重跑收盤匯總失敗：{why}"),
        None => "已重跑收盤匯總".to_string(),
    });
    lines.push(if after.is_empty() {
        "重跑後兩個市場都已完整".to_string()
    } else {
        format!(
            "重跑後仍不完整：{}；請確認來源是否已公布，必要時手動回補當日行情",
            describe(after)
        )
    });
    lines.join("\n")
}

/// 週末或交易所公告的休市日回傳 `true`。
///
/// 休市日 TWSE `MI_INDEX` 只回 `{"stat":"很抱歉，沒有符合條件的資料!"}`，缺少 `tables`
/// 讓解析失敗，每逢週末、假日都記一筆 error（2026-09-25～28 連續四天）。
/// 休市日清單抓取失敗時視為開市，交給 [`aggregate`] 原本「0 筆即略過」的邏輯處理。
async fn is_market_closed(now: DateTime<Local>) -> bool {
    if now.is_weekend() {
        return true;
    }

    let today = now.date_naive();
    match twse::holiday_schedule::visit(today.year()).await {
        Ok(holidays) => holidays.iter().any(|holiday| holiday.date == today),
        Err(why) => {
            tracing::warn!(
                "Failed to fetch TWSE holiday schedule, continuing closing aggregation: {:?}",
                why
            );
            false
        }
    }
}

/// 股票收盤數據匯總。
///
/// 此函式會串起收盤資料回補、缺漏報價補齊、均線、最後交易日報價、
/// 估價、殖利率排行、市值重算與市值變化通知。主要由 [`execute`] 呼叫，
/// 測試環境也會透過手動回補測試檔指定日期執行。
///
/// # Errors
///
/// 任一步驟失敗時會回傳錯誤，呼叫端可依情境記錄或中止後續流程。
pub(crate) async fn aggregate(date: NaiveDate) -> Result<()> {
    //抓取上市櫃公司每日收盤資訊
    let daily_quote_count = backfill::quote::execute(date).await?;
    tracing::info!("抓取上市櫃收盤數據結束:{}", daily_quote_count);

    if daily_quote_count == 0 {
        // 非交易日（假日/收盤 0 筆）：完全略過本次收盤匯總，包含帳戶市值計算，
        // 避免 daily_money_history 系列表被寫入非交易日（如週末）的無效紀錄
        // （部分表格的 upsert SQL 以「當日收盤價精確比對」為前提，非交易日
        // 沒有對應收盤價時會產生全 0 的錯誤資料列）
        tracing::info!("日期 {} 無交易資料，略過本次收盤匯總", date);
        return Ok(());
    }

    // 除權息日的漲跌在交易所行情中是 0，要在均線、統計表與排行之前以官方基準價重算；
    // 失敗只影響當天除權息股的漲跌，不中斷收盤匯總
    match backfill::ex_right_change::execute(date).await {
        Ok(summary) => tracing::info!("除權息日漲跌修正結束:{summary:?}"),
        Err(why) => tracing::error!("除權息日漲跌修正失敗: {why:#}"),
    }

    // 有行情資料才執行的計算（均線、估價、殖利率排行）
    let quote_repo = PgQuoteRepository::new();

    let lack_daily_quotes_count = quote_repo.makeup_for_the_lack_daily_quotes(date).await?;
    tracing::info!(
        "補上當日缺少的每日收盤數據結束:{:#?}",
        lack_daily_quotes_count
    );

    calculation::daily_quotes::calculate_moving_average(date).await?;
    tracing::info!("計算均線結束");

    quote_repo.rebuild_last_daily_quotes().await?;
    tracing::info!("重建 last_daily_quotes 表內的數據結束");

    calculation::estimated_price::calculate_estimated_price(date).await?;
    tracing::info!("計算便宜、合理、昂貴價的估算結束");

    let yield_rank_repo = PgYieldRankRepository::new();
    use crate::domain::yield_rank::repository::YieldRankRepository;
    yield_rank_repo.rebuild_by_date(date).await?;
    tracing::info!("重建 yield_rank 表內的數據結束");

    calculation::money_history::calculate_money_history(date).await?;
    tracing::info!("計算帳戶內市值結束");
    // 技術訊號只供查詢與通知使用，失敗不影響後續的市值變化通知
    match calculation::technical_signal::calculate_technical_signals(date).await {
        Ok(count) => tracing::info!("計算技術訊號結束:{count}"),
        Err(why) => tracing::error!("計算技術訊號失敗: {why:#}"),
    }
    // 未來報酬只供訊號績效統計使用，失敗不影響後續流程
    match calculation::forward_return::calculate_forward_returns(date).await {
        Ok(count) => tracing::info!("回填未來報酬結束:{count}"),
        Err(why) => tracing::error!("回填未來報酬失敗: {why:#}"),
    }

    // 清除記憶與Redis內所有的快取
    TTL.clear();

    // 派發領域事件以非同步處理本日與前一個交易日的市值變化通知
    let dispatcher = crate::app::event::get_global_dispatcher();
    dispatcher
        .dispatch_async(vec![
            crate::domain::events::DomainEvent::MoneyFlowRecalculated {
                date,
                occurred_at: chrono::Local::now(),
            },
        ])
        .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::cache::SHARE;
    use std::time::Duration;

    /// 每日收盤事件主要匯總流程的整合測試。
    #[tokio::test]
    #[ignore]
    async fn test_aggregate() {
        dotenvy::dotenv().ok();
        SHARE.load().await;

        tracing::debug!("開始 event::taiwan_stock::closing::aggregate");

        let current_date = NaiveDate::parse_from_str("2026-04-30", "%Y-%m-%d").unwrap();

        match aggregate(current_date).await {
            Ok(_) => {
                tracing::debug!(
                    "{}",
                    "event::taiwan_stock::closing::aggregate 完成".to_string(),
                );
            }
            Err(why) => {
                tracing::debug!(
                    "Failed to event::taiwan_stock::closing::aggregate because {:?}",
                    why
                );
            }
        }

        tracing::debug!("結束 event::taiwan_stock::closing::aggregate");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    fn count(date: NaiveDate, market_id: i32, traded: i64) -> MarketTradedCount {
        MarketTradedCount {
            date,
            market_id,
            traded,
        }
    }

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, d).unwrap()
    }

    /// 上櫃整天沒抓到（2026-03-17 只剩零量補值列）時判定不完整；上市正常不列入。
    #[test]
    fn shortfalls_flag_a_missing_market() {
        let counts = vec![
            count(day(13), 2, 1_010),
            count(day(13), 4, 880),
            count(day(16), 2, 1_005),
            count(day(16), 4, 875),
            count(day(17), 2, 1_002),
        ];
        assert_eq!(
            shortfalls(day(17), &counts),
            vec![Shortfall {
                market: "上櫃",
                traded: 0,
                baseline: 880,
            }]
        );
    }

    /// 冷門股沒成交造成的小幅波動不算不完整；低於近期最大值一半才算。
    #[test]
    fn shortfalls_tolerate_normal_variation() {
        let counts = vec![
            count(day(16), 2, 1_000),
            count(day(16), 4, 880),
            count(day(17), 2, 950),
            count(day(17), 4, 440),
        ];
        assert!(shortfalls(day(17), &counts).is_empty());

        let partial = vec![
            count(day(16), 4, 880),
            count(day(17), 2, 1_000),
            count(day(17), 4, 439),
        ];
        assert_eq!(shortfalls(day(17), &partial)[0].traded, 439);
    }

    /// 沒有近期資料可比較時，只要當天有成交就算完整；兩個市場都 0 檔則都列出。
    #[test]
    fn shortfalls_without_baseline() {
        assert!(shortfalls(day(17), &[count(day(17), 2, 10), count(day(17), 4, 5)]).is_empty());
        assert_eq!(shortfalls(day(17), &[]).len(), 2);
    }

    /// 依序回傳筆數、記錄重算項目的假倉儲。
    struct DerivedRepo {
        dates: Vec<NaiveDate>,
        counts: std::sync::Mutex<Vec<DerivedCounts>>,
        got_base: std::sync::Mutex<Option<NaiveDate>>,
    }

    impl DerivedRepo {
        fn new(dates: Vec<NaiveDate>, counts: Vec<DerivedCounts>) -> Self {
            Self {
                dates,
                counts: std::sync::Mutex::new(counts),
                got_base: std::sync::Mutex::new(None),
            }
        }
    }

    #[async_trait::async_trait]
    impl ForwardReturnRepository for DerivedRepo {
        async fn fetch_trading_dates(
            &self,
            _until: NaiveDate,
            count: usize,
        ) -> Result<Vec<NaiveDate>> {
            Ok(self.dates.iter().copied().take(count).collect())
        }

        async fn upsert_forward_returns(
            &self,
            _base: NaiveDate,
            _end: NaiveDate,
            _horizon: usize,
        ) -> Result<u64> {
            Ok(0)
        }
    }

    #[async_trait::async_trait]
    impl DerivedDataQuery for DerivedRepo {
        async fn count_derived(&self, _date: NaiveDate, base: NaiveDate) -> Result<DerivedCounts> {
            *self.got_base.lock().expect("測試鎖") = Some(base);
            Ok(self.counts.lock().expect("測試鎖").remove(0))
        }
    }

    /// 由新到舊的連續交易日，第一個是 `end`。
    fn trading_days(end: NaiveDate, count: u64) -> Vec<NaiveDate> {
        (0..count).map(|i| end - chrono::Days::new(i)).collect()
    }

    const COMPLETE: DerivedCounts = DerivedCounts {
        signals: 1800,
        forward_returns: 1900,
    };

    /// 完整時不告警；非交易日不檢查；缺漏時只重算缺的項目，依重算後狀況決定標題。
    #[tokio::test]
    async fn check_derived_recomputes_missing_items() {
        let date = day(13);
        let recomputed = std::sync::Mutex::new(Vec::new());
        let recompute = |derived: Derived| {
            recomputed.lock().expect("測試鎖").push(derived);
            async move {
                match derived {
                    Derived::Signals => Ok(()),
                    Derived::ForwardReturns => Err(anyhow::anyhow!("連線中斷")),
                }
            }
        };

        let complete = DerivedRepo::new(trading_days(date, 6), vec![COMPLETE]);
        assert!(
            check_derived(&complete, date, recompute)
                .await
                .expect("檢查成功")
                .is_none()
        );
        assert_eq!(*complete.got_base.lock().expect("測試鎖"), Some(day(8)));

        let holiday = DerivedRepo::new(trading_days(day(12), 6), Vec::new());
        assert!(
            check_derived(&holiday, date, recompute)
                .await
                .expect("非交易日")
                .is_none()
        );

        let fixed = DerivedRepo::new(
            trading_days(date, 6),
            vec![
                DerivedCounts {
                    signals: 0,
                    forward_returns: 1900,
                },
                COMPLETE,
            ],
        );
        let (title, body) = check_derived(&fixed, date, recompute)
            .await
            .expect("檢查成功")
            .expect("有缺漏");
        assert_eq!(title, "技術訊號／未來報酬已補算");
        assert!(
            body.contains(
                "檢查時缺：技術訊號
已重算"
            ),
            "{body}"
        );

        let still = DerivedRepo::new(
            trading_days(date, 6),
            vec![
                DerivedCounts::default(),
                DerivedCounts {
                    signals: 1800,
                    forward_returns: 0,
                },
            ],
        );
        let (title, body) = check_derived(&still, date, recompute)
            .await
            .expect("檢查成功")
            .expect("有缺漏");
        assert_eq!(title, "技術訊號／未來報酬仍缺漏");
        assert!(body.contains("重算失敗：未來報酬：連線中斷"), "{body}");
        assert_eq!(
            *recomputed.lock().expect("測試鎖"),
            [Derived::Signals, Derived::Signals, Derived::ForwardReturns]
        );
    }

    /// 筆數為 0 的衍生資料才列為缺漏。
    #[test]
    fn missing_derived_lists_zero_counts() {
        assert!(
            missing_derived(DerivedCounts {
                signals: 1800,
                forward_returns: 1900
            })
            .is_empty()
        );
        assert_eq!(
            missing_derived(DerivedCounts::default()),
            [Derived::Signals, Derived::ForwardReturns]
        );
        assert_eq!(
            missing_derived(DerivedCounts {
                signals: 5,
                forward_returns: 0
            }),
            [Derived::ForwardReturns]
        );
    }

    /// 衍生資料告警列出缺漏、重算結果與重算後狀況。
    #[test]
    fn derived_report_describes_before_rerun_and_after() {
        let both = [Derived::Signals, Derived::ForwardReturns];
        assert_eq!(
            derived_report(day(13), &both, &[], &[]),
            "2026-03-13 檢查時缺：技術訊號、未來報酬\n已重算\n重算後都已寫入"
        );
        let failed = derived_report(
            day(13),
            &both,
            &["技術訊號：連線中斷".to_owned()],
            &[Derived::Signals],
        );
        assert_eq!(
            failed,
            "2026-03-13 檢查時缺：技術訊號、未來報酬\n重算失敗：技術訊號：連線中斷\n重算後仍缺：技術訊號；請查看收盤匯總日誌，必要時手動回補"
        );
    }

    /// 告警內容依序說明檢查時的缺口、重跑結果與重跑後狀況。
    #[test]
    fn report_describes_before_rerun_and_after() {
        let missing = vec![Shortfall {
            market: "上櫃",
            traded: 0,
            baseline: 880,
        }];
        assert_eq!(
            report(day(17), &missing, None, &[]),
            "2026-03-17 檢查時：上櫃 0 檔有成交（近期 880 檔）\n已重跑收盤匯總\n重跑後兩個市場都已完整"
        );
        let text = report(day(17), &missing, Some("上櫃收盤資料 0 筆"), &missing);
        assert!(
            text.contains("重跑收盤匯總失敗：上櫃收盤資料 0 筆"),
            "{text}"
        );
        assert!(
            text.contains("重跑後仍不完整：上櫃 0 檔有成交（近期 880 檔）"),
            "{text}"
        );
    }
}
