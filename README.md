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

Mutating data from another thread:

```rust
use scherben_map;
use std::{
    self,
    sync::{Arc, Mutex},
};

fn main() {
    let map = scherben_map::HashMap::<String, Arc<Mutex<String>>, 16>::new_arc();

    map.insert(
        "test".to_string(),
        Arc::new(Mutex::new("result".to_string())),
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

    if *map.get_owned("test".to_string()).unwrap().lock().unwrap()
        != "result + mutation".to_string()
    {
        panic!("inconsistent state");
    }
}
```

# status

This library in under development.

This library is inspired by [concurrent-map](https://github.com/orcaman/concurrent-map/) and [sharded](https://github.com/nkconnor/sharded/).

