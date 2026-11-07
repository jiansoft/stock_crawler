use anyhow::{Result, anyhow};
use chrono::NaiveDate;
use serde::Deserialize;

use crate::{
    core::util::{self, convert::FromValue},
    infra::crawler::share::QfiiDto,
};

/// 櫃買中心「僑外資及陸資持股比例排行表」JSON API（不帶參數即為最近一個交易日）。
///
/// 原本抓 MOPS 的 `server-java/t13sa150_otc` 舊式 HTML，2026-09-29 起該網址一律 404；
/// 那頁解析不到資料列時只會回傳空清單，任務照樣顯示成功，上櫃外資持股因此默默停止更新。
/// 改用資料源頭的櫃買 JSON；每日排程解析不到任何資料時回錯誤（見 [`visit`]）。
const TPEX_QFII_URL: &str = "https://www.tpex.org.tw/www/zh-tw/insti/qfii";

/// 櫃買 QFII API 回應主體。
#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct TpexQfiiResponse {
    /// API 狀態，正常為 `ok`。
    stat: Option<String>,
    /// 資料日期（西元 `YYYYMMDD`）。
    date: Option<String>,
    /// 資料表清單；目前只有一張排行表。
    tables: Vec<TpexQfiiTable>,
}

/// 櫃買 QFII API 回應中的單一資料表。
#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct TpexQfiiTable {
    /// 欄位名稱，用來定位各欄，不依賴固定索引。
    fields: Vec<String>,
    /// 資料列，每格都是字串（數字含千分位逗號、比率含 `%`）。
    data: Vec<Vec<serde_json::Value>>,
}

/// 櫃買某一交易日的外資持股資料。
#[derive(Debug, Clone, PartialEq)]
pub struct OtcQfiiSnapshot {
    /// 資料日期（櫃買回應的 `date`）；不帶日期查詢時是最近一個交易日。
    pub date: NaiveDate,
    /// 各股外資持股資料；指定日期查到休市日時為空。
    pub rows: Vec<QfiiDto>,
}

/// 取得最近一個交易日的上櫃外資及陸資投資持股統計。
///
/// 每日排程用：解析不到任何資料列時回錯誤，避免來源改版時默默更新 0 筆。
pub async fn visit() -> Result<OtcQfiiSnapshot> {
    let response = util::http::get_json::<TpexQfiiResponse>(TPEX_QFII_URL).await?;
    let snapshot = parse_tpex_qfii(response)?;
    ensure_not_empty(&snapshot)?;
    Ok(snapshot)
}

/// 取得指定交易日的上櫃外資持股統計（回補歷史用）。
///
/// 休市日櫃買仍回 `stat: ok` 但沒有資料列，此時回傳空的 `rows`，由呼叫端略過。
pub async fn visit_on(date: NaiveDate) -> Result<OtcQfiiSnapshot> {
    let url = format!(
        "{TPEX_QFII_URL}?date={}&response=json",
        date.format("%Y%%2F%m%%2F%d")
    );
    let response = util::http::get_json::<TpexQfiiResponse>(&url).await?;
    parse_tpex_qfii(response)
}

/// 每日排程不允許空結果：櫃買不帶日期時一定回最近一個交易日的資料。
fn ensure_not_empty(snapshot: &OtcQfiiSnapshot) -> Result<()> {
    if snapshot.rows.is_empty() {
        return Err(anyhow!(
            "TPEx QFII response has no data rows (date={})",
            snapshot.date
        ));
    }
    Ok(())
}

/// 解析櫃買 QFII API 回應（純函式，可用 `testdata/qfii_otc.json` fixture 驗證）。
///
/// 依欄位名稱定位「代號」、「發行股數(A)」、「僑外資及陸資持有股數(C)」、
/// 「僑外資及陸資持股比率(E=C/A)」四欄。`stat` 不是 `ok`、日期無法解析，
/// 或有資料列卻找不到必要欄位時回錯誤；沒有資料列（休市日）回傳空的 `rows`。
fn parse_tpex_qfii(response: TpexQfiiResponse) -> Result<OtcQfiiSnapshot> {
    if let Some(stat) = response.stat.as_deref()
        && !stat.eq_ignore_ascii_case("ok")
    {
        return Err(anyhow!("TPEx QFII API returned stat={stat}"));
    }

    let date = response
        .date
        .as_deref()
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y%m%d").ok())
        .ok_or_else(|| anyhow!("TPEx QFII response has no valid date: {:?}", response.date))?;

    let Some(table) = response.tables.into_iter().next() else {
        return Ok(OtcQfiiSnapshot {
            date,
            rows: Vec::new(),
        });
    };
    if table.data.is_empty() {
        return Ok(OtcQfiiSnapshot {
            date,
            rows: Vec::new(),
        });
    }

    let column = |name: &str| {
        table
            .fields
            .iter()
            .position(|field| field.starts_with(name))
            .ok_or_else(|| anyhow!("TPEx QFII table has no `{name}` column: {:?}", table.fields))
    };
    let symbol_col = column("代號")?;
    let issued_col = column("發行股數")?;
    let held_col = column("僑外資及陸資持有股數")?;
    let percentage_col = column("僑外資及陸資持股比率")?;

    let rows = table
        .data
        .iter()
        .filter_map(|row| {
            let stock_symbol = row.get(symbol_col)?.get_string(None);
            if stock_symbol.is_empty() {
                return None;
            }
            Some(QfiiDto {
                stock_symbol,
                issued_share: row.get(issued_col)?.get_i64(None),
                shares_held: row.get(held_col)?.get_i64(None),
                share_holding_percentage: row.get(percentage_col)?.get_decimal(None),
            })
        })
        .collect();

    Ok(OtcQfiiSnapshot { date, rows })
}

#[cfg(test)]
mod tests;
