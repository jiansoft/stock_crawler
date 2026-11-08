//! 持股穿透曝險：把持有 ETF 的市值依前十大持股權重拆到成分股，與直接持股合併。
//!
//! ETF 只知道前十大持股，十大以外的部分無法拆解，計入 `ExposureReport::uncovered_value`；
//! 反向、期貨、部分債券 ETF 沒有前十大持股，整檔都無法拆解。台股成分股以代號比對，
//! 債券等沒有代號的持股以名稱比對。

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;

/// 一檔持有部位。
#[derive(Debug, Clone, PartialEq)]
pub struct ExposurePosition {
    /// 證券代號。
    pub stock_symbol: String,
    /// 名稱。
    pub name: String,
    /// 市值（股數 × 最新收盤價，元）；沒有收盤價時為 `None`。
    pub market_value: Option<f64>,
    /// 是否為 ETF（清單頁快照上有的）。
    pub is_etf: bool,
}

/// 一檔 ETF 的一檔前十大持股。
#[derive(Debug, Clone, PartialEq)]
pub struct EtfConstituent {
    /// ETF 代號。
    pub etf_symbol: String,
    /// 持股資料日期。
    pub as_of: NaiveDate,
    /// 台股成分股代號；債券等沒有代號為空字串。
    pub holding_symbol: String,
    /// 名稱。
    pub holding_name: String,
    /// 比例（%）。
    pub weight: f64,
}

/// 經由某檔 ETF 持有的金額。
#[derive(Debug, Clone, PartialEq)]
pub struct ExposureSource {
    /// ETF 代號。
    pub etf_symbol: String,
    /// 金額（元）。
    pub value: f64,
}

/// 一檔成分股的合計曝險。
#[derive(Debug, Clone, PartialEq)]
pub struct Exposure {
    /// 台股代號；沒有代號的持股為空字串。
    pub symbol: String,
    /// 名稱（直接持股的名稱優先）。
    pub name: String,
    /// 直接持有的市值（元）。
    pub direct_value: f64,
    /// 經由 ETF 持有的金額（元），依金額由大到小。
    pub sources: Vec<ExposureSource>,
}

impl Exposure {
    /// 經由 ETF 持有的合計金額；沒有經由 ETF 時為 0（`Iterator::sum` 對空的 f64 會得到 -0.0，
    /// JSON 會輸出成 `-0.0`）。
    pub fn etf_value(&self) -> f64 {
        self.sources
            .iter()
            .fold(0.0, |total, source| total + source.value)
    }

    /// 直接加上經由 ETF 的合計金額。
    pub fn total_value(&self) -> f64 {
        self.direct_value + self.etf_value()
    }
}

/// 一檔持有 ETF 的拆解程度。
#[derive(Debug, Clone, PartialEq)]
pub struct EtfLookThrough {
    /// ETF 代號。
    pub stock_symbol: String,
    /// 名稱。
    pub name: String,
    /// 市值（元）。
    pub market_value: f64,
    /// 前十大持股的資料日期；沒有前十大時為 `None`。
    pub holdings_as_of: Option<NaiveDate>,
    /// 前十大持股的權重合計（%），即可拆解的比例。
    pub covered_weight: f64,
}

/// 持股穿透的結果。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExposureReport {
    /// 有收盤價的部位市值合計（元）。
    pub total_value: f64,
    /// 其中 ETF 的市值合計（元）。
    pub etf_value: f64,
    /// ETF 前十大以外、無法拆解的金額（元）。
    pub uncovered_value: f64,
    /// 持有的 ETF，依市值由大到小。
    pub etfs: Vec<EtfLookThrough>,
    /// 各成分股的合計曝險，依合計金額由大到小。
    pub exposures: Vec<Exposure>,
    /// 沒有收盤價、不列入計算的代號。
    pub unpriced: Vec<String>,
}

/// 成分股的比對鍵：有代號用代號，沒有用名稱。
fn key(symbol: &str, name: &str) -> String {
    if symbol.is_empty() {
        format!("name:{name}")
    } else {
        symbol.to_owned()
    }
}

/// 取出或新增一檔成分股的曝險。
fn entry<'a>(
    exposures: &'a mut HashMap<String, Exposure>,
    symbol: &str,
    name: &str,
) -> &'a mut Exposure {
    exposures
        .entry(key(symbol, name))
        .or_insert_with(|| Exposure {
            symbol: symbol.to_owned(),
            name: name.to_owned(),
            direct_value: 0.0,
            sources: Vec::new(),
        })
}

/// 把 ETF 市值依前十大持股拆到成分股，與直接持股合併。
pub fn look_through(
    positions: &[ExposurePosition],
    constituents: &[EtfConstituent],
) -> ExposureReport {
    let mut report = ExposureReport::default();
    let mut exposures: HashMap<String, Exposure> = HashMap::new();
    for position in positions {
        let Some(value) = position.market_value else {
            report.unpriced.push(position.stock_symbol.clone());
            continue;
        };
        report.total_value += value;
        if !position.is_etf {
            let exposure = entry(&mut exposures, &position.stock_symbol, &position.name);
            exposure.direct_value += value;
            exposure.name.clone_from(&position.name);
            continue;
        }
        report.etf_value += value;
        let mut look = EtfLookThrough {
            stock_symbol: position.stock_symbol.clone(),
            name: position.name.clone(),
            market_value: value,
            holdings_as_of: None,
            covered_weight: 0.0,
        };
        for constituent in constituents
            .iter()
            .filter(|c| c.etf_symbol == position.stock_symbol)
        {
            look.holdings_as_of = Some(constituent.as_of);
            look.covered_weight += constituent.weight;
            entry(
                &mut exposures,
                &constituent.holding_symbol,
                &constituent.holding_name,
            )
            .sources
            .push(ExposureSource {
                etf_symbol: position.stock_symbol.clone(),
                value: value * constituent.weight / 100.0,
            });
        }
        // 權重合計偶有略超過 100（四捨五入），無法拆解的部分不為負。
        report.uncovered_value += value * (100.0 - look.covered_weight).max(0.0) / 100.0;
        report.etfs.push(look);
    }
    report
        .etfs
        .sort_by(|a, b| b.market_value.total_cmp(&a.market_value));
    report.exposures = exposures
        .into_values()
        .map(|mut exposure| {
            exposure.sources.sort_by(|a, b| b.value.total_cmp(&a.value));
            exposure
        })
        .collect();
    report.exposures.sort_by(|a, b| {
        b.total_value()
            .total_cmp(&a.total_value())
            .then_with(|| a.symbol.cmp(&b.symbol))
            .then_with(|| a.name.cmp(&b.name))
    });
    report
}

/// 持股穿透需要的讀取合約。
#[async_trait]
pub trait EtfExposureQuery: Send + Sync {
    /// 未賣出的持有部位（同代號合併）與市值；`member_id` 為 0 時為全部會員合計。
    async fn fetch_exposure_positions(&self, member_id: i64) -> Result<Vec<ExposurePosition>>;

    /// 指定 ETF 最新一期的前十大持股。
    async fn fetch_etf_constituents(&self, etf_symbols: &[String]) -> Result<Vec<EtfConstituent>>;
}

/// 讀取持有部位與 ETF 前十大持股後計算持股穿透。
///
/// # Errors
///
/// 倉儲讀取失敗時回傳錯誤。
pub async fn build_report(repo: &impl EtfExposureQuery, member_id: i64) -> Result<ExposureReport> {
    let positions = repo.fetch_exposure_positions(member_id).await?;
    let etfs: Vec<String> = positions
        .iter()
        .filter(|position| position.is_etf && position.market_value.is_some())
        .map(|position| position.stock_symbol.clone())
        .collect();
    let constituents = if etfs.is_empty() {
        Vec::new()
    } else {
        repo.fetch_etf_constituents(&etfs).await?
    };
    Ok(look_through(&positions, &constituents))
}

#[cfg(test)]
mod tests;
