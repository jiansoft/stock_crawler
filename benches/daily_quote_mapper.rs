use std::collections::HashMap;

use chrono::NaiveDate;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

use stock_crawler::app::backfill::acl::quote::QuoteAclMapper;
use stock_crawler::infra::crawler::share::DailyQuoteDto;
use stock_crawler::infra::crawler::twse::quote::ListedResponse;

const FIXTURE: &str = include_str!("../tests/fixtures/twse_quote.json");

fn bench_acl_chain(c: &mut Criterion) {
    let response: ListedResponse = serde_json::from_str(FIXTURE).unwrap();
    let table = &response.tables[0];
    let rows = table.data.as_ref().unwrap();
    let fields = table.fields.as_ref().unwrap();
    let field_map: HashMap<&str, usize> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| (f.as_str(), i))
        .collect();
    let date = NaiveDate::from_ymd_opt(2026, 6, 25).unwrap();
    // from_with_map 已改為 Result（typed 解析錯誤）；fixture 是真實 TWSE 資料，
    // 理應全數可解析，失敗代表 fixture 或解析器壞掉。
    let dtos: Vec<DailyQuoteDto> = rows
        .iter()
        .map(|row| {
            DailyQuoteDto::from_with_map(row, &field_map, date).expect("fixture row should parse")
        })
        .collect();

    c.bench_function("acl_chain_100rows", |b| {
        b.iter(|| {
            dtos.iter()
                .map(|dto| {
                    let cmd = QuoteAclMapper::from_dto(black_box(dto));
                    QuoteAclMapper::from_command(&cmd)
                })
                .collect::<Vec<_>>()
        })
    });
}

criterion_group!(benches, bench_acl_chain);
criterion_main!(benches);
