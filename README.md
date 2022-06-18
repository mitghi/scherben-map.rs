# scherben-map.rs

Concurrent Sharded HashMap for Rust.

[scherben-map](https://github.com/mitghi/scherben-map.rs) splits the HashMap into N shards, each protected by their own Read/Write lock.

#### example

Using atomically reference counted HashMap:

```rust
use scherben_map;
use std;

fn main() {
    let map = scherben_map::HashMap::<&str, &str, /*shard count*/ 8>::new_arc();

    map.insert("Hallo", "Welt");

    {
        let (map_a, map_b) = (map.clone(), map.clone());
        let handle_a = std::thread::spawn(move || {
            println!("{}", map_a.get_owned("Hallo").unwrap());
        });

        let handle_b = std::thread::spawn(move || {
            println!("{}", map_b.get_owned("Hallo").unwrap());
        });

        _ = handle_a.join();
        _ = handle_b.join();
    }

    // get all key/value pairs
    let pairs = map.pairs();

    // get all keys
    let keys = map.keys();
}
```

Extract key/value pairs from shards using threadpool and channels.

```rust
use crossbeam::{
    channel::{bounded, unbounded, Receiver, Sender},
    select,
    sync::WaitGroup,
};
use scoped_threadpool::Pool;
use std::hash::Hash;

extern crate scoped_threadpool;

#[rustfmt::skip]
fn collect<K: Send + Sync + Clone + 'static, V: Send + Sync + Clone + 'static, const N: usize>(map: &scherben_map::HashMap<K, V, N>, pool: &mut scoped_threadpool::Pool) -> Vec<(K, V)>
where
    K: Hash + Eq + scherben_map::IKey<K>,
{
    let (result_tx, result_rx): (Sender<Vec<(K, V)>>, Receiver<Vec<(K, V)>>) = unbounded();

    {
        let result_tx = result_tx.clone();
        pool.scoped(move |s| {
            let (buffer_tx, buffer_rx): (Sender<Vec<(K, V)>>, Receiver<Vec<(K, V)>>) = bounded(64);
            let wg = WaitGroup::new();
            {
                let buffer_rx = buffer_rx.clone();
                s.execute(move || {
                    let mut result: Vec<(K, V)> = Vec::new();
                    loop {
                        select! {
                            recv(buffer_rx) -> msg => {
                                match msg {
                    Ok(value) => result.extend(value),
                    Err(_) => break,
                                }
                            }
                        }
                    }
                    _ = result_tx.send(result);
                });
            }
            {
                for shard_slot in map.into_iter() {
                    let wg = wg.clone();
                    let shard_handle = shard_slot.clone();
                    let buffer_tx = buffer_tx.clone();
                    s.execute(move || {
                        let mut buffer: Vec<(K, V)> = Vec::new();
                        let _shard = shard_handle.read();
                        (*_shard).fill_pairs_into(&mut buffer);
                        _ = buffer_tx.send(buffer);

                        drop(wg);
                    });
                }
            }
            wg.wait();
            drop(buffer_rx);
        });
    }
    match result_rx.recv() {
        Ok(result) => result,
        Err(_) => Vec::new(),
    }
}

#[rustfmt::skip]
pub fn keys<K: Send + Sync + Clone + 'static, V: Send + Sync + Clone + 'static, const N: usize>(map: &scherben_map::HashMap<K, V, N>, pool: &mut scoped_threadpool::Pool) -> Vec<K>
where
    K: Hash + Eq + scherben_map::IKey<K>,
{
    collect(map, pool).iter().map(|x| x.0.clone()).collect()
}

pub fn pairs<K: Send + Sync + Clone + 'static, V: Send + Sync + Clone + 'static, const N: usize>(
    map: &scherben_map::HashMap<K, V, N>,
    pool: &mut scoped_threadpool::Pool,
) -> Vec<(K, V)>
where
    K: Hash + Eq + scherben_map::IKey<K> + Clone,
    V: Clone,
{
    collect(&map, pool)
}

fn main() {
    let map: scherben_map::HashMap<String, i64, 8> = Default::default();
    let mut pool = Pool::new(num_cpus::get().try_into().unwrap());

    for i in 0..1000 {
        let item = format!("Hello, Many Worlds {}!", i);
        map.insert(item, i);
    }

    let map_keys = pairs(&map, &mut pool);

    if map_keys.len() != 1000 {
        panic!("inconsistent state");
    }
}
```

# status

This library in under development.

This library is inspired by [concurrent-map](https://github.com/orcaman/concurrent-map/) and [sharded](https://github.com/nkconnor/sharded/).
