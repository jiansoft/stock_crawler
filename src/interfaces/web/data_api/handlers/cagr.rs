//! `/market/cagr-ranking` 與 `/market/cagr-ranking/{stock_symbol}` 兩個
//! 每日 CAGR handlers、其參數白名單與領域實體到 API DTO 的轉換（M4）。

use axum::{
    Json,
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::{database_error, error_response, parse_optional_date, repository_error};
use crate::domain::performance::entity::{
    CagrMetric, CagrPeriod, PRINCIPAL, SimulationOutcome, StockCagr as DomainStockCagr,
};
use crate::domain::performance::query::{
    CagrRankingItem as DomainCagrRankingItem, CagrRankingQuery, CagrSortKey,
};
use crate::domain::performance::repository::CagrRepository;
use crate::infra::database;
use crate::infra::database::repository::performance::PgCagrRepository;
use crate::interfaces::web::data_api::dto::{
    CagrCoverageInfo, CagrPeriodItem, CagrRankingItem, CagrRankingParams, CagrRankingResponse,
    CagrSummary, CagrSymbolParams, CagrSymbolResponse, ErrorBody,
};

/// 每日 CAGR 排行（M4）。
///
/// 以固定投入 [`PRINCIPAL`] 元模擬各期間、各口徑的報酬，回傳單頁排行、
/// 分頁總筆數與樣本涵蓋統計。三個設計取捨值得注意：
///
/// - **所有金額與比率序列化為字串**：後端是 `rust_decimal`，轉成 JSON
///   number 會經過 f64 而掉精度；`principal` 與各種計數是整數，維持 number。
/// - **資料不足的項目回傳 `null` 而非從 `items` 省略**：「查得到但算不出來」
///   與「查不到」是兩件不同的事，前端據此渲染灰化列。
/// - **長期間不提供純價格口徑**：`Y5`／`Y10` 搭配 `metric=price` 回 422。
///
/// # Errors
///
/// 參數不合法或長期間要求純價格口徑回 422；整張表尚無計算結果回 404；
/// 驗證失敗回 401；倉儲查詢失敗時記錄內部錯誤並回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/cagr-ranking", tag = "data-api", params(CagrRankingParams), responses((status = 200, body = CagrRankingResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn cagr_ranking(Query(params): Query<CagrRankingParams>) -> Response {
    let period = match parse_cagr_period(params.period.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let metric = match parse_cagr_metric(params.metric.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    // 由領域層判定，不在此硬寫期間清單——判準改變時只需改一處。
    if metric == CagrMetric::Price && !period.supports_price_metric() {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "長期間不提供純價格口徑：近十年每年皆有 134–216 檔股票配股,\
             忽略配股在長期間的低估幅度顯著,請改用 total 或 reinvested",
        );
    }
    let sort = match params.sort.as_deref() {
        // 未指定時依期間長度決定：短期間年化會被放大約四倍，統計意義薄弱。
        None => CagrSortKey::default_for(period),
        Some(code) => match CagrSortKey::from_code(code) {
            Some(value) => value,
            None => {
                return error_response(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "sort 必須為 cagr 或 total_return",
                );
            }
        },
    };
    let market_id = match cagr_market_id(params.market.as_deref().unwrap_or("all")) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    if params.stock_industry_id.is_some_and(|value| value <= 0) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "stock_industry_id 必須為正整數",
        );
    }
    let keyword = match normalize_cagr_keyword(params.keyword.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let limit = params.limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 200");
    }
    let offset = params.offset.unwrap_or(0);
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };

    let repository = PgCagrRepository::new();
    let date = match resolve_cagr_date(&repository, date).await {
        Ok(Some(value)) => value,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "查無 CAGR 計算結果"),
        Err(error) => return repository_error(error),
    };

    let query = CagrRankingQuery {
        date,
        period,
        metric,
        sort,
        market_id,
        industry_id: params.stock_industry_id,
        keyword,
        include_incomplete: params.include_incomplete.unwrap_or(true),
        limit: i64::from(limit),
        offset: i64::from(offset),
    };
    let page = match repository.fetch_ranking_page(&query).await {
        Ok(value) => value,
        Err(error) => return repository_error(error),
    };

    // 表頭的期初日／年數取本頁可算項目中最常見者：同一基準日的絕大多數
    // 股票會對齊到同一個交易日，少數因停牌順延的個股不應決定表頭。
    let base_date = representative_base_date(&page.items);
    let years = base_date.and_then(|target| {
        page.items
            .iter()
            .find(|item| item.cagr.data_complete && item.cagr.base_date == Some(target))
            .and_then(|item| item.cagr.years)
    });

    Json(CagrRankingResponse {
        period: period.code().to_owned(),
        metric: metric.code().to_owned(),
        sort: sort.code().to_owned(),
        date: date.to_string(),
        base_date: base_date.map(|value| value.to_string()),
        years: decimal_ratio(years),
        principal: PRINCIPAL,
        total: page.total,
        coverage: CagrCoverageInfo {
            universe: page.coverage.universe,
            counted: page.coverage.counted,
            coverage_ratio: format!("{:.4}", page.coverage.coverage_ratio()),
            incomplete: page.coverage.incomplete,
            anomaly_flagged: page.coverage.anomaly_flagged,
            survivorship_note: period.requires_coverage_disclosure(),
        },
        summary: CagrSummary {
            positive: page.coverage.positive,
            // 分母是可算檔數而非母體，直接取領域方法避免各處各算一份。
            positive_ratio: format!("{:.4}", page.coverage.positive_ratio()),
        },
        items: page
            .items
            .into_iter()
            .map(|item| cagr_ranking_item(item, metric))
            .collect(),
    })
    .into_response()
}

/// 單一個股在指定基準日的全部八個期間 CAGR（M4）。
///
/// 與排行不同，這裡一律回傳全部期間（含資料不足者），讓個股頁能同時呈現
/// 「三個月」到「十年」的完整輪廓；資料不足的期間各數值欄位為 `null`。
///
/// # Errors
///
/// `metric` 或 `date` 不合法回 422；未知代號或該股該日無計算結果回 404；
/// 驗證失敗回 401；倉儲查詢失敗回不含 SQL 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/cagr-ranking/{stock_symbol}", tag = "data-api", params(("stock_symbol" = String, Path, description = "股票代號"), CagrSymbolParams), responses((status = 200, body = CagrSymbolResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn cagr_by_symbol(
    Path(stock_symbol): Path<String>,
    Query(params): Query<CagrSymbolParams>,
) -> Response {
    let metric = match parse_cagr_metric(params.metric.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };
    let date = match parse_optional_date(params.date.as_deref()) {
        Ok(value) => value,
        Err(message) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, message),
    };

    // 先確認代號存在，讓「代號打錯」與「這檔股票尚未計算」都是 404 但訊息不同。
    let profile: Result<Option<(String, i32)>, _> =
        sqlx::query_as(r#"SELECT "Name", stock_industry_id FROM stocks WHERE stock_symbol = $1"#)
            .bind(&stock_symbol)
            .fetch_optional(database::get_connection())
            .await;
    let (name, industry_id) = match profile {
        Ok(Some(value)) => value,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "找不到股票代號"),
        Err(error) => return database_error(error),
    };

    let repository = PgCagrRepository::new();
    let date = match resolve_cagr_date(&repository, date).await {
        Ok(Some(value)) => value,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "查無 CAGR 計算結果"),
        Err(error) => return repository_error(error),
    };
    let records = match repository.fetch_by_symbol(date, &stock_symbol).await {
        Ok(value) => value,
        Err(error) => return repository_error(error),
    };
    if records.is_empty() {
        return error_response(StatusCode::NOT_FOUND, "查無此股票的 CAGR 計算結果");
    }

    Json(CagrSymbolResponse {
        stock_symbol,
        name: Some(name.clone()),
        metric: metric.code().to_owned(),
        date: date.to_string(),
        principal: PRINCIPAL,
        items: records
            .into_iter()
            .map(|record| cagr_period_item(record, &name, industry_id, metric))
            .collect(),
    })
    .into_response()
}

/// 解析 `period` 查詢參數；未提供時預設 `Y1`。
fn parse_cagr_period(value: Option<&str>) -> Result<CagrPeriod, &'static str> {
    CagrPeriod::from_code(value.unwrap_or("Y1"))
        .ok_or("period 必須為 M3、M6、Y1、Y1H、Y2、Y3、Y5、Y7 或 Y10")
}

/// 解析 `metric` 查詢參數；未提供時預設主指標 `total`。
fn parse_cagr_metric(value: Option<&str>) -> Result<CagrMetric, &'static str> {
    CagrMetric::from_code(value.unwrap_or("total"))
        .ok_or("metric 必須為 price、total 或 reinvested")
}

/// 將 `market` 參數轉成倉儲層的市場編號篩選。
///
/// 與其他排行 endpoint 不同，`all` 在此是「不篩選」（`None`）而非展開成
/// `IN (2, 4)`：CAGR 的母體與涵蓋率統計以整個 `(基準日, 期間)` 為範圍，
/// 若這裡偷偷縮小集合，畫面上的檔數就對不上 `coverage.universe`。
/// 除三個代號外亦接受市場編號，供前端直接指定興櫃等其他市場。
fn cagr_market_id(market: &str) -> Result<Option<i32>, &'static str> {
    match market {
        "all" => Ok(None),
        "twse" => Ok(Some(2)),
        "tpex" => Ok(Some(4)),
        other => other
            .parse::<i32>()
            .ok()
            .filter(|value| *value > 0)
            .map(Some)
            .ok_or("market 必須為 all、twse、tpex 或正整數市場編號"),
    }
}

/// 正規化關鍵字：去除前後空白，空字串視為未指定。
fn normalize_cagr_keyword(keyword: Option<&str>) -> Result<Option<String>, &'static str> {
    let Some(trimmed) = keyword.map(str::trim) else {
        return Ok(None);
    };
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > 50 {
        return Err("keyword 長度必須介於 1 至 50 字元");
    }
    Ok(Some(trimmed.to_owned()))
}

/// 決定實際查詢的基準日：未指定時取最新一個已完成計算的日期。
async fn resolve_cagr_date(
    repository: &PgCagrRepository,
    date: Option<NaiveDate>,
) -> anyhow::Result<Option<NaiveDate>> {
    match date {
        Some(value) => Ok(Some(value)),
        None => repository.fetch_latest_date().await,
    }
}

/// 取出指定口徑的模擬結果；該口徑不可算時為 `None`。
fn cagr_outcome(record: &DomainStockCagr, metric: CagrMetric) -> Option<SimulationOutcome> {
    match metric {
        CagrMetric::Price => record.price,
        CagrMetric::Total => record.total,
        CagrMetric::Reinvested => record.reinvested,
    }
}

/// 將金額或比率序列化為固定四位小數的字串。
///
/// 一律走字串是刻意的：`rust_decimal` 轉 JSON number 會經過 f64，
/// 十年期的期末總值在 f64 下會出現尾數漂移。
fn decimal_ratio(value: Option<Decimal>) -> Option<String> {
    value.map(|number| format!("{number:.4}"))
}

/// 將股數序列化為固定八位小數的字串（配股會產生零股）。
fn decimal_shares(value: Option<Decimal>) -> Option<String> {
    value.map(|number| format!("{number:.8}"))
}

/// 將排行榜的領域項目轉成 API DTO。
fn cagr_ranking_item(item: DomainCagrRankingItem, metric: CagrMetric) -> CagrRankingItem {
    let outcome = cagr_outcome(&item.cagr, metric);
    CagrRankingItem {
        rank: item.rank,
        stock_symbol: item.cagr.stock_symbol,
        name: item.name,
        stock_industry_id: item.industry_id,
        base_date: item.cagr.base_date.map(|value| value.to_string()),
        first_quote_date: item.cagr.first_quote_date.map(|value| value.to_string()),
        shortfall_days: item.cagr.shortfall_days,
        base_price: decimal_ratio(item.cagr.base_price),
        end_price: decimal_ratio(item.cagr.end_price),
        end_shares: decimal_shares(outcome.map(|value| value.end_shares)),
        cash_received: decimal_ratio(outcome.map(|value| value.cash_received)),
        end_value: decimal_ratio(outcome.map(|value| value.end_value)),
        total_return_pct: decimal_ratio(outcome.map(|value| value.total_return_pct)),
        cagr_pct: decimal_ratio(outcome.map(|value| value.cagr_pct)),
        dividend_events: item.cagr.dividend_events,
        data_complete: item.cagr.data_complete,
        has_anomaly: item.cagr.has_anomaly,
    }
}

/// 將個股單一期間的領域實體轉成 API DTO。
fn cagr_period_item(
    record: DomainStockCagr,
    name: &str,
    industry_id: i32,
    metric: CagrMetric,
) -> CagrPeriodItem {
    let outcome = cagr_outcome(&record, metric);
    CagrPeriodItem {
        period: record.period.code().to_owned(),
        stock_symbol: record.stock_symbol,
        name: name.to_owned(),
        stock_industry_id: industry_id,
        base_date: record.base_date.map(|value| value.to_string()),
        first_quote_date: record.first_quote_date.map(|value| value.to_string()),
        shortfall_days: record.shortfall_days,
        base_price: decimal_ratio(record.base_price),
        end_price: decimal_ratio(record.end_price),
        end_shares: decimal_shares(outcome.map(|value| value.end_shares)),
        cash_received: decimal_ratio(outcome.map(|value| value.cash_received)),
        end_value: decimal_ratio(outcome.map(|value| value.end_value)),
        total_return_pct: decimal_ratio(outcome.map(|value| value.total_return_pct)),
        cagr_pct: decimal_ratio(outcome.map(|value| value.cagr_pct)),
        years: decimal_ratio(record.years),
        dividend_events: record.dividend_events,
        data_complete: record.data_complete,
        has_anomaly: record.has_anomaly,
        survivorship_note: record.period.requires_coverage_disclosure(),
    }
}

/// 取本頁可算項目中最常見的期初交易日。
///
/// 同票數時取較晚的日期，讓結果與資料列順序無關（可重現）。
fn representative_base_date(items: &[DomainCagrRankingItem]) -> Option<NaiveDate> {
    let mut counts: std::collections::HashMap<NaiveDate, usize> = std::collections::HashMap::new();
    for item in items.iter().filter(|item| item.cagr.data_complete) {
        if let Some(value) = item.cagr.base_date {
            *counts.entry(value).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(date, count)| (*count, *date))
        .map(|(date, _)| date)
}

#[cfg(test)]
mod tests {
    //! 每日 CAGR endpoint 的純函式測試（M4）。
    //!
    //! 全部不需要資料庫：參數解析、白名單驗證與 Decimal → 字串序列化都是
    //! 純運算，而這三者正是前端契約最容易在重構時被悄悄改掉的部分。

    use chrono::NaiveDate;
    use rust_decimal_macros::dec;

    use super::{
        DomainCagrRankingItem, DomainStockCagr, cagr_market_id, cagr_period_item,
        cagr_ranking_item, decimal_ratio, decimal_shares, normalize_cagr_keyword,
        parse_cagr_metric, parse_cagr_period, representative_base_date,
    };
    use crate::domain::performance::entity::{CagrMetric, CagrPeriod, SimulationOutcome};
    use crate::domain::performance::query::CagrSortKey;

    /// 建立一筆資料齊全的領域結果。
    fn complete(symbol: &str, period: CagrPeriod, base_date: NaiveDate) -> DomainStockCagr {
        DomainStockCagr {
            date: NaiveDate::from_ymd_opt(2026, 8, 6).unwrap(),
            stock_symbol: symbol.to_owned(),
            period,
            base_date: Some(base_date),
            base_price: Some(dec!(178)),
            end_price: Some(dec!(1340)),
            years: Some(dec!(10.002740)),
            price: None,
            total: Some(SimulationOutcome {
                end_shares: dec!(56.17977528),
                cash_received: dec!(6797.7528),
                end_value: dec!(82078.6517),
                total_return_pct: dec!(720.7865),
                cagr_pct: dec!(23.4318),
            }),
            reinvested: None,
            first_quote_date: NaiveDate::from_ymd_opt(2015, 3, 2),
            shortfall_days: Some(0),
            data_complete: true,
            has_anomaly: false,
            dividend_events: 38,
        }
    }

    /// 建立一筆資料不足的領域結果（所有數值欄位皆為 None）。
    fn incomplete(symbol: &str, period: CagrPeriod) -> DomainStockCagr {
        DomainStockCagr {
            date: NaiveDate::from_ymd_opt(2026, 8, 6).unwrap(),
            stock_symbol: symbol.to_owned(),
            period,
            base_date: None,
            base_price: None,
            end_price: None,
            years: None,
            price: None,
            total: None,
            reinvested: None,
            first_quote_date: NaiveDate::from_ymd_opt(2024, 5, 1),
            shortfall_days: None,
            data_complete: false,
            has_anomaly: false,
            dividend_events: 0,
        }
    }

    /// 建立排行榜項目。
    fn ranking_item(cagr: DomainStockCagr, rank: Option<i64>) -> DomainCagrRankingItem {
        DomainCagrRankingItem {
            cagr,
            name: "台積電".to_owned(),
            industry_id: 24,
            rank,
        }
    }

    /// 十年期範例使用的期初交易日。
    fn base_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2016, 8, 5).unwrap()
    }

    /// `period` 與 `metric` 的預設值與白名單必須與前端契約一致。
    #[test]
    fn test_period_and_metric_defaults_and_whitelist() {
        assert_eq!(parse_cagr_period(None), Ok(CagrPeriod::Y1));
        assert_eq!(parse_cagr_metric(None), Ok(CagrMetric::Total));
        for period in CagrPeriod::ALL {
            assert_eq!(parse_cagr_period(Some(period.code())), Ok(period));
        }
        for metric in CagrMetric::ALL {
            assert_eq!(parse_cagr_metric(Some(metric.code())), Ok(metric));
        }
        // 大小寫與別名一律不接受，避免前後端對「y1」是否合法各自解讀。
        assert!(parse_cagr_period(Some("y1")).is_err());
        assert!(parse_cagr_period(Some("Y8")).is_err());
        assert!(parse_cagr_metric(Some("TOTAL")).is_err());
        assert!(parse_cagr_metric(Some("cash")).is_err());
    }

    /// 未指定 `sort` 時，短期間必須改用區間總報酬——年化會被放大約四倍。
    #[test]
    fn test_default_sort_key_follows_period_length() {
        assert_eq!(
            CagrSortKey::default_for(CagrPeriod::M3),
            CagrSortKey::TotalReturn
        );
        assert_eq!(
            CagrSortKey::default_for(CagrPeriod::M6),
            CagrSortKey::TotalReturn
        );
        for period in [CagrPeriod::Y1, CagrPeriod::Y5, CagrPeriod::Y10] {
            assert_eq!(CagrSortKey::default_for(period), CagrSortKey::Cagr);
        }
        assert_eq!(
            CagrSortKey::from_code("total_return"),
            Some(CagrSortKey::TotalReturn)
        );
        assert_eq!(CagrSortKey::from_code("cagr"), Some(CagrSortKey::Cagr));
        assert_eq!(CagrSortKey::from_code("CAGR"), None);
    }

    /// 長期間不得提供純價格口徑；判準取自領域層而非在 handler 硬寫。
    #[test]
    fn test_long_periods_reject_price_metric() {
        for period in [CagrPeriod::Y5, CagrPeriod::Y10] {
            assert!(!period.supports_price_metric(), "{period:?} 應拒絕純價格");
            assert!(period.requires_coverage_disclosure());
        }
        for period in [CagrPeriod::M3, CagrPeriod::Y1, CagrPeriod::Y3] {
            assert!(period.supports_price_metric());
            assert!(!period.requires_coverage_disclosure());
        }
    }

    /// `market` 白名單：`all` 代表不篩選，其餘映射到市場編號。
    #[test]
    fn test_market_parameter_whitelist() {
        assert_eq!(cagr_market_id("all"), Ok(None));
        assert_eq!(cagr_market_id("twse"), Ok(Some(2)));
        assert_eq!(cagr_market_id("tpex"), Ok(Some(4)));
        assert_eq!(cagr_market_id("5"), Ok(Some(5)));
        assert!(cagr_market_id("0").is_err());
        assert!(cagr_market_id("-1").is_err());
        assert!(cagr_market_id("emerging").is_err());
    }

    /// 關鍵字去空白後為空即視為未指定，過長則拒絕。
    #[test]
    fn test_keyword_normalization() {
        assert_eq!(normalize_cagr_keyword(None), Ok(None));
        assert_eq!(normalize_cagr_keyword(Some("   ")), Ok(None));
        assert_eq!(
            normalize_cagr_keyword(Some("  台積  ")),
            Ok(Some("台積".to_owned()))
        );
        assert!(normalize_cagr_keyword(Some(&"台".repeat(51))).is_err());
    }

    /// 金額與股數必須是固定小數位的字串——JSON number 會經過 f64 掉精度。
    #[test]
    fn test_decimal_serializes_as_fixed_scale_string() {
        assert_eq!(decimal_ratio(Some(dec!(178))), Some("178.0000".to_owned()));
        assert_eq!(
            decimal_ratio(Some(dec!(82078.6517))),
            Some("82078.6517".to_owned())
        );
        assert_eq!(
            decimal_ratio(Some(dec!(10.002740))),
            Some("10.0027".to_owned())
        );
        assert_eq!(
            decimal_shares(Some(dec!(56.17977528))),
            Some("56.17977528".to_owned())
        );
        assert_eq!(decimal_ratio(None), None);
        assert_eq!(decimal_shares(None), None);
    }

    /// 可算項目的每個欄位都要對到正確來源，且名次原樣帶出。
    #[test]
    fn test_ranking_item_maps_selected_metric() {
        let item = ranking_item(complete("2330", CagrPeriod::Y10, base_date()), Some(1));
        let dto = cagr_ranking_item(item, CagrMetric::Total);
        assert_eq!(dto.rank, Some(1));
        assert_eq!(dto.stock_symbol, "2330");
        assert_eq!(dto.name, "台積電");
        assert_eq!(dto.stock_industry_id, 24);
        assert_eq!(dto.base_date.as_deref(), Some("2016-08-05"));
        assert_eq!(dto.first_quote_date.as_deref(), Some("2015-03-02"));
        assert_eq!(dto.shortfall_days, Some(0));
        assert_eq!(dto.base_price.as_deref(), Some("178.0000"));
        assert_eq!(dto.end_price.as_deref(), Some("1340.0000"));
        assert_eq!(dto.end_shares.as_deref(), Some("56.17977528"));
        assert_eq!(dto.cash_received.as_deref(), Some("6797.7528"));
        assert_eq!(dto.end_value.as_deref(), Some("82078.6517"));
        assert_eq!(dto.total_return_pct.as_deref(), Some("720.7865"));
        assert_eq!(dto.cagr_pct.as_deref(), Some("23.4318"));
        assert_eq!(dto.dividend_events, 38);
        assert!(dto.data_complete);
        assert!(!dto.has_anomaly);
    }

    /// 未計算的口徑（此處為純價格）必須是 null，不可退回主指標的數字。
    #[test]
    fn test_ranking_item_missing_metric_is_null() {
        let item = ranking_item(complete("2330", CagrPeriod::Y10, base_date()), Some(1));
        let dto = cagr_ranking_item(item, CagrMetric::Price);
        assert!(dto.end_shares.is_none());
        assert!(dto.cash_received.is_none());
        assert!(dto.end_value.is_none());
        assert!(dto.total_return_pct.is_none());
        assert!(dto.cagr_pct.is_none());
        // 期初／期末價屬於該列本身，與口徑無關，仍應有值。
        assert!(dto.base_price.is_some());
    }

    /// 資料不足的項目要留在清單裡並回 null，而不是被省略。
    #[test]
    fn test_incomplete_item_keeps_row_with_null_fields() {
        let mut item = ranking_item(incomplete("6666", CagrPeriod::Y10), None);
        item.name = "測試股".to_owned();
        item.industry_id = 3;
        let dto = cagr_ranking_item(item, CagrMetric::Total);
        assert_eq!(dto.rank, None, "資料不足不得佔名次");
        assert_eq!(dto.stock_symbol, "6666");
        assert!(!dto.data_complete);
        for field in [
            &dto.base_date,
            &dto.base_price,
            &dto.end_price,
            &dto.end_shares,
            &dto.cash_received,
            &dto.end_value,
            &dto.total_return_pct,
            &dto.cagr_pct,
        ] {
            assert!(field.is_none(), "資料不足時所有數值欄位必須是 null");
        }
        assert!(dto.shortfall_days.is_none());
        // 查得到但算不出來：仍保留最早報價日，供前端說明為何算不出來。
        assert_eq!(dto.first_quote_date.as_deref(), Some("2024-05-01"));
    }

    /// 個股項目多一個 period、帶 years 與該期間的揭露旗標。
    #[test]
    fn test_period_item_carries_period_and_disclosure_flag() {
        let dto = cagr_period_item(
            complete("2330", CagrPeriod::Y10, base_date()),
            "台積電",
            24,
            CagrMetric::Total,
        );
        assert_eq!(dto.period, "Y10");
        assert_eq!(dto.years.as_deref(), Some("10.0027"));
        assert_eq!(dto.stock_industry_id, 24);
        assert!(dto.survivorship_note);

        let short = cagr_period_item(
            complete("2330", CagrPeriod::M3, base_date()),
            "台積電",
            24,
            CagrMetric::Total,
        );
        assert_eq!(short.period, "M3");
        assert!(!short.survivorship_note);
    }

    /// 表頭期初日取可算項目中最常見者，且不受資料不足項目干擾。
    #[test]
    fn test_representative_base_date_ignores_incomplete_rows() {
        let common = base_date();
        let delayed = NaiveDate::from_ymd_opt(2016, 8, 22).unwrap();
        let items = vec![
            ranking_item(complete("a", CagrPeriod::Y10, common), Some(1)),
            ranking_item(complete("b", CagrPeriod::Y10, delayed), Some(2)),
            ranking_item(complete("c", CagrPeriod::Y10, common), Some(3)),
            ranking_item(incomplete("d", CagrPeriod::Y10), None),
        ];
        assert_eq!(representative_base_date(&items), Some(common));
        assert_eq!(representative_base_date(&[]), None);
        assert_eq!(
            representative_base_date(&[ranking_item(incomplete("d", CagrPeriod::Y10), None)]),
            None
        );
    }
}
