use std::collections::HashMap;

use chrono::NaiveDate;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

use stock_crawler::infra::crawler::share::DailyQuoteDto;
use stock_crawler::infra::crawler::twse::quote::ListedResponse;

const FIXTURE: &str = include_str!("../tests/fixtures/twse_quote.json");

fn bench_json_deser(c: &mut Criterion) {
    c.bench_function("twse_json_deser_100rows", |b| {
        b.iter(|| serde_json::from_str::<ListedResponse>(black_box(FIXTURE)).unwrap())
    });
}

fn bench_dto_map(c: &mut Criterion) {
    let response: ListedResponse = serde_json::from_str(FIXTURE).unwrap();
    let table = &response.tables[0];
    let rows = table.data.as_ref().unwrap();
    let fields = table.fields.as_ref().unwrap();
    let field_map: HashMap<&str, usize> =
        fields.iter().enumerate().map(|(i, f)| (f.as_str(), i)).collect();
    let date = NaiveDate::from_ymd_opt(2026, 6, 25).unwrap();

    c.bench_function("twse_dto_map_100rows", |b| {
        b.iter(|| {
            rows.iter()
                .map(|row| DailyQuoteDto::from_with_map(black_box(row), &field_map, date))
                .collect::<Vec<_>>()
        })
    });
}

criterion_group!(benches, bench_json_deser, bench_dto_map);
criterion_main!(benches);
