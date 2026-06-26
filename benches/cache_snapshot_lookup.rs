use std::collections::HashMap;

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use rust_decimal::Decimal;

use stock_crawler::infra::cache::{RealtimeSnapshot, Share};

fn bench_get_snapshot(c: &mut Criterion) {
    let share = Share::new();
    let mut snapshots = HashMap::new();
    for i in 1101u32..1201 {
        let sym = i.to_string();
        snapshots.insert(sym.clone(), RealtimeSnapshot::new(sym, Decimal::new(500, 0)));
    }
    share.set_stock_snapshots(snapshots);

    c.bench_function("cache_get_snapshot_hit", |b| {
        b.iter(|| share.get_stock_snapshot(black_box("1101")))
    });

    c.bench_function("cache_get_snapshot_miss", |b| {
        b.iter(|| share.get_stock_snapshot(black_box("9999")))
    });
}

fn bench_set_snapshot_price(c: &mut Criterion) {
    let share = Share::new();
    let mut snap = RealtimeSnapshot::new("2330".to_string(), Decimal::new(500, 0));
    snap.last_close = Decimal::new(500, 0);
    let mut init = HashMap::new();
    init.insert("2330".to_string(), snap);
    share.set_stock_snapshots(init);

    c.bench_function("cache_set_snapshot_price", |b| {
        b.iter(|| {
            share.set_stock_snapshot_price(
                black_box("2330".to_string()),
                black_box(Decimal::new(501, 0)),
            )
        })
    });
}

criterion_group!(benches, bench_get_snapshot, bench_set_snapshot_price);
criterion_main!(benches);
