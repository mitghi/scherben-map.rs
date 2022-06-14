use crossbeam::{
    channel::{bounded, unbounded, Receiver, Sender},
    select,
    sync::WaitGroup,
};
use fnv::FnvHasher;
use hashbrown::raw::RawTable;
use scoped_threadpool::Pool;
use std::{
    borrow::Borrow,
    fmt,
    hash::{Hash, Hasher},
};

extern crate scoped_threadpool;

type RwLock<T> = parking_lot_utils::RwLock<T>;

pub trait Key<K>: Hash + Eq {
    fn key(&self) -> &K;
}

pub trait IKey<K> {
    fn as_bytes(&self) -> &[u8];
}

struct RwKey<'a, K: IKey<K>> {
    key: &'a K,
}

pub struct HashMap<K, V, const N: usize> {
    shards: [RwLock<Shard<K, V>>; N],
    shards_size: u64,
}

struct Shard<K, V> {
    table: RawTable<(K, V)>,
}

impl<K: Clone + Send + Sync, V: Clone + Send + Sync, const N: usize> Default for HashMap<K, V, N> {
    fn default() -> Self {
        Self::with_shard(N.next_power_of_two())
    }
}

impl<K: Clone + Send + Sync, V: Clone + Send + Sync> Shard<K, V> {
    pub fn get<'a>(&'a self, key: &K, hash: u64) -> Option<&'a V>
    where
        K: Hash + Eq + IKey<K>,
    {
        match self.table.get(hash, move |x| key.eq(x.0.borrow())) {
            Some(&(_, ref value)) => Some(value),
            None => None,
        }
    }

    pub fn insert(&mut self, key: &K, hash: u64, value: V)
    where
        K: Hash + Eq + IKey<K> + Clone,
        V: Clone,
    {
        self.table.insert(hash, (key.clone(), value), |x| {
            make_hash(x.0.borrow().as_bytes())
        });
    }

    pub fn remove(&mut self, key: &K, hash: u64) -> Option<V>
    where
        K: Hash + Eq + IKey<K> + Clone,
        V: Clone,
    {
        match self.table.remove_entry(hash, move |x| key.eq(x.0.borrow())) {
            Some((_, value)) => Some(value),
            None => None,
        }
    }

    fn fill_pairs_into(&self, buffer: &mut Vec<(K, V)>) {
        unsafe {
            for entry in self.table.iter() {
                let value = entry.as_ref().clone();
                buffer.push((value.0.clone(), value.1.clone()));
            }
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.table.len()
    }
}

impl<'a, K: Clone + Send + Sync, V: Clone + Send + Sync, const N: usize> HashMap<K, V, N> {
    pub fn new() -> Self {
        Self::with_shard(N.next_power_of_two())
    }

    pub fn new_arc() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self::new())
    }

    pub fn with_shard(shards_size: usize) -> Self {
        let shards = std::iter::repeat(|| RawTable::with_capacity(shards_size))
            .map(|f| f())
            .take(shards_size)
            .map(|table| RwLock::new(Shard { table }))
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        Self {
            shards,
            shards_size: shards_size as u64,
        }
    }

    pub fn get_owned(&'a self, key: K) -> Option<V>
    where
        K: Hash + Eq + IKey<K>,
        V: Clone,
    {
        let hash: u64 = make_hash(key.as_bytes());
        let bin = hash % self.shards_size;
        let shard = match self.shards.get(bin as usize) {
            Some(lock) => lock.read(),
            None => panic!("index out of bounds"),
        };
        match shard.get(&key, hash) {
            Some(result) => Some(result.clone()),
            None => None,
        }
    }

    pub fn insert(&self, key: K, value: V)
    where
        K: Hash + Eq + IKey<K>,
    {
        let hash: u64 = make_hash(key.as_bytes());
        let bin = hash % self.shards_size;
        let mut shard = match self.shards.get(bin as usize) {
            Some(lock) => lock.write(),
            None => panic!("index out of bounds"),
        };
        shard.insert(&key, hash, value);
    }

    pub fn remove(&self, key: K) -> Option<V>
    where
        K: Hash + Eq + IKey<K>,
    {
        let hash: u64 = make_hash(key.as_bytes());
        let bin = hash % self.shards_size;
        let mut shard = match self.shards.get(bin as usize) {
            Some(lock) => lock.write(),
            None => panic!("index out of bounds"),
        };

        shard.remove(&key, hash)
    }

    pub fn contains(&self, key: K) -> bool
    where
        K: Hash + Eq + IKey<K>,
    {
        let hash: u64 = make_hash(key.as_bytes());
        let bin = hash % self.shards_size;
        let shard = match self.shards.get(bin as usize) {
            Some(lock) => lock.read(),
            None => panic!("index out of bounds"),
        };
        match shard.get(&key, hash) {
            Some(_) => true,
            None => false,
        }
    }

    pub fn len(&self) -> usize {
        self.shards.iter().map(|x| x.read().len()).sum()
    }

    #[rustfmt::skip]
    fn collect<'b>(&'b self) -> Vec<(K, V)>
    where
        K: Hash + Eq + IKey<K>,
    {
        let mut pool = Pool::new(num_cpus::get().try_into().unwrap());
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
                    for shard_slot in self.shards.iter() {
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
    pub fn keys<'b>(&'b self) -> Vec<K>
    where
        K: Hash + Eq + IKey<K>,
    {
	self.collect().iter().map(|x| x.0.clone()).collect()
    }

    pub fn pairs<'b>(&'b self) -> Vec<(K, V)>
    where
        K: Hash + Eq + IKey<K> + Clone,
        V: Clone,
    {
        self.collect()
    }
}

impl<K, V> fmt::Debug for Shard<K, V> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> fmt::Result {
        write!(fmt, "Shard{{table: [{}]}}", self.table.len())
    }
}

impl<'a, K: IKey<K> + std::cmp::PartialEq> PartialEq for RwKey<'a, K> {
    fn eq(&self, other: &Self) -> bool {
        *self.key == *other.key
    }
}

impl<'a, K: IKey<K> + std::cmp::PartialEq> Eq for RwKey<'a, K> {}

impl<K> IKey<K> for String {
    fn as_bytes(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl<K> IKey<K> for &str {
    fn as_bytes(&self) -> &[u8] {
        (*self).as_bytes()
    }
}

impl<'a, K: IKey<K>> Hash for RwKey<'a, K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(make_hash(self.key.as_bytes()));
        state.finish();
    }
}

impl<'a, K: IKey<K> + Eq> Key<K> for RwKey<'a, K> {
    fn key(&self) -> &K {
        &self.key
    }
}

fn make_hash(key: &[u8]) -> u64 {
    let mut hasher: Box<dyn Hasher> = Box::new(FnvHasher::default());
    hasher.write(key);
    hasher.finish()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn two_threads_performing_read_write() {
        let map: std::sync::Arc<HashMap<String, std::sync::Arc<std::sync::Mutex<String>>, 16>> =
            HashMap::<String, std::sync::Arc<std::sync::Mutex<String>>, 16>::new_arc();
        map.insert(
            "test".to_string(),
            std::sync::Arc::new(std::sync::Mutex::new("result".to_string())),
        );
        {
            let map_a = map.clone();
            let map_b = map.clone();
            let handle_a = std::thread::spawn(move || {
                match map_a.get_owned("test".to_string()) {
                    Some(result) => {
                        let mut value = result.lock().unwrap();
                        value.push_str(" + mutation");
                        println!("result: {}", &value);
                    }
                    None => println!("found nothing ( from thread A )"),
                };
            });

            let handle_b = std::thread::spawn(move || match map_b.get_owned("test".to_string()) {
                Some(result) => {
                    let value = result.lock().unwrap();
                    println!("result: {}", &value);
                }
                None => println!("found nothing ( from thread B )"),
            });

            handle_a.join().unwrap();
            handle_b.join().unwrap();
        }

        assert_eq!(
            *map.get_owned("test".to_string()).unwrap().lock().unwrap(),
            "result + mutation".to_string()
        )
    }

    #[test]
    fn write_and_remove() {
        let map: HashMap<&str, i64, 8> = Default::default();
        map.insert("test", 6);
        assert_eq!(map.get_owned("test").unwrap(), 6);

        map.remove("test");
        assert_eq!(map.get_owned("test"), None);
    }

    #[test]
    fn get_keys_and_pairs() {
        let map: HashMap<String, i64, 8> = Default::default();

        for i in 0..1000 {
            let item = format!("Hallo, Welt {}!", i);
            map.insert(item, i);
        }

        assert_eq!(map.len(), 1000);
        assert_eq!(map.keys().len(), 1000);
    }
}
