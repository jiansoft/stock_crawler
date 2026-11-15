//! 個人業務資料表清單的掃描測試：`etc/sql/*.sql` 裡每張含 `member_id` 的表都必須登記。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use regex::Regex;

use super::*;

/// 解析 SQL 文字，回傳「表名 → 是否含 member_id 欄位」。
///
/// 涵蓋 `CREATE TABLE … ( … )` 的欄位定義與 `ALTER TABLE … ADD COLUMN [IF NOT EXISTS] member_id`。
/// 表名去掉 `public.` 與雙引號；註解行先移除，避免說明文字裡的 member_id 被誤判。
fn tables_in(sql: &str) -> BTreeMap<String, bool> {
    let without_comments: String = sql
        .lines()
        .map(|line| line.split("--").next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    let create = Regex::new(
        r#"(?i)create\s+table\s+(?:if\s+not\s+exists\s+)?(?:public\.)?"?([A-Za-z_][A-Za-z0-9_]*)"?\s*\("#,
    )
    .expect("CREATE TABLE 正規式");
    let add_member = Regex::new(
        r#"(?is)alter\s+table\s+(?:only\s+)?(?:public\.)?"?([A-Za-z_][A-Za-z0-9_]*)"?\s+add\s+column\s+(?:if\s+not\s+exists\s+)?member_id\b"#,
    )
    .expect("ADD COLUMN 正規式");
    let column = Regex::new(r"(?m)^\s*member_id\s").expect("欄位正規式");

    let mut tables = BTreeMap::new();
    for captures in create.captures_iter(&without_comments) {
        let name = captures[1].to_owned();
        let body_start = captures.get(0).map_or(0, |m| m.end());
        let mut depth = 1;
        let mut end = body_start;
        for (offset, ch) in without_comments[body_start..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                end = body_start + offset;
                break;
            }
        }
        let has_member = column.is_match(&without_comments[body_start..end]);
        *tables.entry(name).or_insert(false) |= has_member;
    }
    for captures in add_member.captures_iter(&without_comments) {
        tables.insert(captures[1].to_owned(), true);
    }
    tables
}

/// 讀取 `etc/sql/` 下所有 `.sql`，合併每張表的結果。
fn scan_etc_sql() -> BTreeMap<String, bool> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("etc/sql");
    let mut tables = BTreeMap::new();
    for entry in std::fs::read_dir(&dir).expect("讀取 etc/sql") {
        let path = entry.expect("目錄項目").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("sql") {
            continue;
        }
        let sql = std::fs::read_to_string(&path).expect("讀取 SQL 檔");
        for (name, has_member) in tables_in(&sql) {
            *tables.entry(name).or_insert(false) |= has_member;
        }
    }
    tables
}

/// 解析器本身：建表欄位、補欄位、註解與其他欄位名稱都判斷正確。
#[test]
fn parser_detects_member_id_columns() {
    let sql = r#"
-- create table public.commented (member_id bigint)
CREATE TABLE IF NOT EXISTS public.new_personal (
    serial      bigserial PRIMARY KEY,
    member_id   bigint NOT NULL, -- 註解
    amount      numeric(18, 4) DEFAULT 0
);
create table public.market_only
(
    stock_symbol varchar(24) not null,
    note text -- 這裡提到 member_id 不算欄位
);
ALTER TABLE public."Legacy"
    ADD COLUMN IF NOT EXISTS member_id bigint NOT NULL DEFAULT 1;
"#;
    let tables = tables_in(sql);
    assert_eq!(tables.get("new_personal"), Some(&true));
    assert_eq!(tables.get("market_only"), Some(&false));
    assert_eq!(tables.get("Legacy"), Some(&true));
    assert!(!tables.contains_key("commented"));
}

/// etc/sql 裡每張含 member_id 的表都登記在個人業務表或 IAM 表清單；
/// 新增含 member_id 的表而沒有登記時，這個測試失敗。
#[test]
fn every_table_with_member_id_is_registered() {
    let registered: BTreeSet<&str> = PERSONAL_BUSINESS_TABLES
        .iter()
        .chain(IAM_TABLES.iter())
        .copied()
        .collect();
    let unregistered: Vec<String> = scan_etc_sql()
        .into_iter()
        .filter(|(name, has_member)| *has_member && !registered.contains(name.as_str()))
        .map(|(name, _)| name)
        .collect();
    assert!(
        unregistered.is_empty(),
        "含 member_id 但未登記的表：{unregistered:?}。請加進 domain::personal_data 的清單、\
         計畫 4.0 第 ⑦ 區，個人業務表另需更新 etc/sql/member_db_roles.sql 的 personal_tables"
    );
}

/// 清單裡的每張表都真的存在於 etc/sql（避免改名後清單沒跟上）。
#[test]
fn registered_tables_exist_in_etc_sql() {
    let tables = scan_etc_sql();
    let missing: Vec<&str> = PERSONAL_BUSINESS_TABLES
        .iter()
        .chain(IAM_TABLES.iter())
        .copied()
        .filter(|name| !tables.contains_key(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "清單中但 etc/sql 找不到的表：{missing:?}"
    );
}

/// 角色檔的 personal_tables（決定 stock_readonly 不可讀的表）與這份清單一致。
#[test]
fn db_roles_personal_tables_match_registry() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("etc/sql/member_db_roles.sql");
    let sql = std::fs::read_to_string(path).expect("讀取角色檔");
    let start = sql
        .find("personal_tables CONSTANT text[] := ARRAY[")
        .expect("角色檔有 personal_tables");
    let block = &sql[start..];
    let block = &block[..block.find("];").expect("personal_tables 結尾")];
    let quoted = Regex::new(r"'([a-z_]+)'").expect("名稱正規式");
    let in_roles: BTreeSet<&str> = quoted
        .captures_iter(block)
        .filter_map(|captures| captures.get(1).map(|m| m.as_str()))
        .collect();
    let registry: BTreeSet<&str> = PERSONAL_BUSINESS_TABLES.iter().copied().collect();
    assert_eq!(in_roles, registry);
}
