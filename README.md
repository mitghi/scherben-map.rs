# scherben-map.rs

Concurrent Sharded HashMap for Rust. 

[scherben-map](https://github.com/mitghi/scherben-map.rs) splits the HashMap into N shards, each protected by their own Read/Write lock.

# status

This library in under development.

This library is inspired by [concurrent-map](https://github.com/orcaman/concurrent-map/) and [sharded](https://github.com/nkconnor/sharded/).

