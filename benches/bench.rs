use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

use sharded;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, RwLock};
use std::thread;

pub trait Adaptor<K, V> {
    fn insert(&self, k: K, v: V);
    fn get(&self, k: K) -> Option<V>;
}

pub struct MapRwLock<K, V>(RwLock<HashMap<K, V>>);
pub struct MapSharded<K, V>(sharded::Map<K, V>);
pub struct MapScherben<K, V>(scherben_map::HashMap<K, V, 32>);

impl<K, V> MapRwLock<K, V> {
    pub fn new() -> Self {
        Self(RwLock::new(HashMap::new()))
    }
}

impl<K: std::fmt::Debug, V: std::fmt::Debug> MapSharded<K, V> {
    pub fn new() -> Self {
        Self(sharded::Map::new())
    }
}

impl<K: Send + Sync + Clone, V: Send + Sync + Clone> MapScherben<K, V> {
    pub fn new() -> Self {
        Self(scherben_map::HashMap::new())
    }
}

impl<K: Hash + Eq, V: Clone> Adaptor<K, V> for MapRwLock<K, V> {
    fn insert(&self, k: K, v: V) {
        self.0.write().unwrap().insert(k, v);
    }
    fn get(&self, k: K) -> Option<V> {
        let _lock = self.0.read().unwrap();
        match _lock.get(&k) {
            Some(ref v) => Some((*v).clone()),
            None => None,
        }
    }
}

impl<K: Hash + Eq, V: Clone> Adaptor<K, V> for MapSharded<K, V> {
    fn insert(&self, k: K, v: V) {
        self.0.insert(k, v);
    }
    fn get(&self, k: K) -> Option<V> {
        self.0.get_owned(&k)
    }
}

impl<
        K: Hash + Eq + Clone + Send + Sync + scherben_map::IKey<K> + 'static,
        V: Hash + Eq + Clone + Send + Sync + 'static,
    > Adaptor<K, V> for MapScherben<K, V>
{
    fn insert(&self, k: K, v: V) {
        self.0.insert(k, v);
    }
    fn get(&self, k: K) -> Option<V> {
        self.0.get_owned(k)
    }
}

pub fn proc(map: Arc<dyn Adaptor<String, i64> + Sync + Send + 'static>, threads: usize) {
    let mut joins = Vec::with_capacity(threads);

    {
        let map = map.clone();
        let join = thread::spawn(move || {
            for i in 0..100 {
                (*map).insert(format!("test {}", i), i);
            }
        });
        joins.push(join);
    }

    for i in 0..100 {
        let map = map.clone();
        let join = thread::spawn(move || {
            for i in 0..100 {
                (*map).get("hello 1".to_string());
            }
        });
        joins.push(join);
    }
    for join in joins {
        join.join().unwrap();
    }
}

pub fn criterion_benchmark(c: &mut Criterion) {
    let map: Arc<dyn Adaptor<String, i64> + Send + Sync> = Arc::new(MapRwLock::new());
    for i in 0..1000 {
        map.insert(format!("hello {}", i), i);
    }
    c.bench_function("rwlock_bench 100", move |b| {
        let map = map.clone();
        b.iter(move || proc(map.clone(), 100));
    });
}

pub fn criterion_benchmark2(c: &mut Criterion) {
    let map: Arc<dyn Adaptor<String, i64> + Send + Sync> = Arc::new(MapSharded::new());
    for i in 0..1000 {
        map.insert(format!("hello {}", i), i);
    }
    c.bench_function("sharded_bench 100", move |b| {
        let map = map.clone();
        b.iter(move || proc(map.clone(), 100));
    });
}

pub fn criterion_benchmark3(c: &mut Criterion) {
    let map: Arc<dyn Adaptor<String, i64> + Send + Sync> = Arc::new(MapScherben::new());
    for i in 0..1000 {
        map.insert(format!("hello {}", i), i);
    }
    c.bench_function("scherben_bench 100", move |b| {
        let map = map.clone();
        b.iter(move || proc(map.clone(), 100));
    });
}

pub fn bench_maps(c: &mut Criterion) {
    let rw_map: Arc<dyn Adaptor<String, i64> + Send + Sync> = Arc::new(MapRwLock::new());
    let sharded_map: Arc<dyn Adaptor<String, i64> + Send + Sync> = Arc::new(MapSharded::new());
    let scherben_map: Arc<dyn Adaptor<String, i64> + Send + Sync> = Arc::new(MapScherben::new());
    for i in 0..1000 {
        rw_map.insert(format!("hello {}", i), i);
        sharded_map.insert(format!("hello {}", i), i);
        scherben_map.insert(format!("hello {}", i), i);
    }

    let mut group = c.benchmark_group("Maps");

    for i in 0..4 {
        {
            let map = rw_map.clone();
            group.bench_with_input(BenchmarkId::new("RW Map", i), &i, move |b, i| {
                b.iter(|| proc(map.clone(), *i));
            });
        }

        {
            let map = sharded_map.clone();
            group.bench_with_input(BenchmarkId::new("Sharded Map", i), &i, move |b, i| {
                b.iter(|| proc(map.clone(), *i));
            });
        }

        {
            let map = scherben_map.clone();
            group.bench_with_input(BenchmarkId::new("Scherben Map", i), &i, move |b, i| {
                b.iter(|| proc(map.clone(), *i));
            });
        }
    }

    group.finish();
}

criterion_group!(
    benches,
    criterion_benchmark,
    criterion_benchmark2,
    criterion_benchmark3,
    bench_maps,
);
criterion_main!(benches);
