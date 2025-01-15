An implementation of [tunasync](https://github.com/tuna/tunasync) in Rust, so it‘s called rtsync(rust implemented tunasync).

一个 [tunasync](https://github.com/tuna/tunasync) 的 rust 实现版本，r（rust implemented）t（tuna）sync。

值得注意的修改：

 1. cli、log 全汉化；

 2. 使用 [Rocket](https://github.com/rwf2/Rocket) 作为 web 服务器；

 3. 使用 [tokio](https://github.com/tokio-rs/tokio) 作为异步运行时；

 4. 支持的数据库类型 ： [redis](https://github.com/redis/redis)、[leveldb](https://github.com/google/leveldb)、 [rocksdb](https://github.com/facebook/rocksdb)；

    

还未重构的部分：

1. 对 cgroup 的支持；

   

此仓库创建的目的仅是尝试rust重构代码的可行性，还不能对质量做任何保证，所以除个人学习外请使用原版tunasync。



