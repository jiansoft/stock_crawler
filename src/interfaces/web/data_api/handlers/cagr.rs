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
mod tests;
