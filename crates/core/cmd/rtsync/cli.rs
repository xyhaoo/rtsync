use clap::{arg, Arg, ArgAction, Command};
use internal as rtsync;

pub(crate) fn build_cli() -> Command{
    Command::new("rtsync")
        .version(rtsync::version::VERSION)
        .about("rtsync镜像任务管理工具 一个tunasync(https://github.com/tuna/tunasync)的rust实现版本")
        .subcommand(
            Command::new("manager")
                .about("启动 rtsync manager")
                .alias("m") // 添加简写别名
                .args(&[
                    arg!(-c --config <FILE> "从 `FILE` 中加载 manager 配置"),
                    arg!(--addr <ADDR> "manager 监听 `ADDR` 端口"),
                    arg!(--port <PORT> "manager 绑定 `PORT`"),
                    arg!(--cert <FILE> "使用 `FILE` 中的SSL证书"),
                    arg!(--key <FILE> "使用 `FILE` 中的SSL密钥"),
                    Arg::new("db-file").long("db-file")
                        .value_name("FILE").action(ArgAction::Set)
                        .help("使用 `FILE` 中的数据库文件"),
                    Arg::new("db-type").long("db-type")
                        .value_name("TYPE").action(ArgAction::Set)
                        .help("使用 `TYPE` 作为数据库类型"),
                    arg!(-v --verbose "启用详细日志记录"),
                    arg!(--debug "以 debug 模式启动 manager"),
                    Arg::new("with-systemd").long("with-systemd")
                        .action(ArgAction::SetFalse)
                        .help("启用系统兼容的日志记录"),
                    arg!(--pidfile <PID_FILE> "使用 `PID_FILE` 作为 manager 的pid文件")
                        .default_value("/run/rtsync/rtsync.manager.pid"),
                ])
                .arg_required_else_help(false)
        )
        .subcommand(
            Command::new("worker")
                .about("启动 rtsync worker")
                .alias("w") // 添加简写别名
                .args(&[
                    arg!(-c --config <FILE> "从 `FILE` 中加载 worker 配置"),
                    arg!(-v --verbose "启用详细日志记录"),
                    arg!(--debug "以 debug 模式启动 worker"),
                    Arg::new("with-systemd").long("with-systemd")
                        .action(ArgAction::SetFalse)
                        .help("启用系统兼容的日志记录"),
                    arg!(--pidfile <PID_FILE> "使用 `PID_FILE` 作为 worker 的pid文件")
                        .default_value("/run/rtsync/rtsync.worker.pid"),
                    Arg::new("prof-path").long("prof-path")
                        .value_name("PROF_PATH").action(ArgAction::Set)
                        .help("启用性能分析，将结果保存到 `PROF_FILE`"),
                ])
                .arg_required_else_help(false)
        )
        .arg_required_else_help(true)
}