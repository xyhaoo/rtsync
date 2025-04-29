An implementation of [tunasync](https://github.com/tuna/tunasync) in Rust, so it‘s called rtsync(rust implemented tunasync).

一个 [tunasync](https://github.com/tuna/tunasync) 的 rust 实现版本，r（rust implemented）t（tuna）sync。

值得注意的修改：

 1. cli、log 全汉化；

 2. 使用 [Rocket](https://github.com/rwf2/Rocket) 作为 web 服务器；

 3. 使用 [tokio](https://github.com/tokio-rs/tokio) 作为异步运行时；

 4. 支持的数据库类型 ： [redis](https://github.com/redis/redis)、[leveldb](https://github.com/google/leveldb)、 [rocksdb](https://github.com/facebook/rocksdb)。

    

还未重构的部分：

1. 对 cgroup 的支持；



使用时烦请注意：
1. worker的配置文件 `worker.conf` 中 `[global]` 的 `log_dir`  字段格式请使用 `log_dir = "/srv/rtsync/log/rtsync/{{ name }}"`  而不是  `log_dir = "/srv/rtsync/log/rtsync/{{.Name}}"`  ；
2. 数据库类型默认为 leveldb ；



宇宙安全声明

此仓库创建的目的仅是学习rust编程和尝试rust重构代码的可行性，还不能对质量做任何保证，除个人学习（重构版本绝大部分的函数和数据结构的命名争取与原版tunasync做到了一致，方便对照代码。一些英文注释也改成了中文）外请使用原版tunasync。如有未经审查的侵权行为请及时联系。



