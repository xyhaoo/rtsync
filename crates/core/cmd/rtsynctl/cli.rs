use clap::{arg, Arg, ArgAction, Command};
use internal as rtsync;

pub(crate) fn build_cli() -> Command{
    // 用来配置client
    let common_flags = [
        arg!(-c --config <FILE> "从 `FILE` 中加载 manager 配置，而不是从 ~/.config/rtsync/ctl.conf 和 /etc/rtsync/ctl.conf"),
        arg!(-m --manager <ADDRESS> "设定 manager 服务器地址"),
        arg!(-p --port <PORT> "设定 manager 服务器端口号"),
        Arg::new("ca-cert").long("ca-cert")
            .value_name("CERT").action(ArgAction::Set)
            .help("指定`CERT`为信任根CA证书文件路径"),
        arg!(-v --verbose "启用详细日志记录"),
        arg!(--debug "开启调试日志")];

    // 向manager服务器发送start命令时使用
    let force_start_flag = [
        arg!(-f --force "忽略并发限制"),
    ];
    
    Command::new("rtsynctl")
        .version(rtsync::version::VERSION)
        .about("rtsync manager的控制客户端 一个tunasynctl(https://github.com/tuna/tunasync)的rust实现版本")
        .subcommand(
            Command::new("list")
                .about("列出 workers 的同步任务")
                .arg(Arg::new("WORKERS")
                    .help("指定 `WORKERS`")
                    .num_args(0..)
                    .conflicts_with("all"))  // 参数可以为0个或多个，与all冲突
                .args(&common_flags)
                .args(&[
                    arg!(-a --all "列出 workers 的所有同步任务"),
                    arg!(-s --status <STATUS> "根据提供的 `STATUS` 过滤输出"),
                    arg!(-f --format <FORMAT> "使用 Tera 的 `FORMAT` 模版格式化输出"),
                ])
                .arg_required_else_help(true)
        )
        .subcommand(
            Command::new("flush")
                .about("刷新已禁用的同步任务")
                .args(&common_flags)
                .arg_required_else_help(false)
        )
        .subcommand(
            Command::new("workers")
                .about("列出 workers")
                .args(&common_flags)
                .arg_required_else_help(false)
        )
        .subcommand(
            Command::new("rm-worker")
                .about("删除一个 worker")
                .args(&common_flags)
                .arg(arg!(<WORKER> "指定 `WORKER`"))
                .arg_required_else_help(true)
        )
        .subcommand(
            Command::new("set-size")
                .about("设置镜像大小")
                .arg(arg!(<WORKER> "指定 `WORKER`"))  // 必须要求被获得的参数，不提供会报错
                .arg(arg!(<MIRROR> "指定任务镜像为 `MIRROR`"))
                .arg(arg!(<SIZE> "指定 `SIZE`"))
                .args(&common_flags)
                .arg_required_else_help(true)
        )
        .subcommand(
            Command::new("start")
                .about("开始一个同步任务")
                .arg(arg!(<WORKER> "指定 `WORKER`"))
                .arg(arg!(<MIRROR> "指定 `MIRROR`"))
                .args(&common_flags)
                .args(&force_start_flag)
                .arg_required_else_help(true)
        )
        .subcommand(
            Command::new("stop")
                .about("暂停一个同步任务")
                .arg(arg!(<WORKER> "指定 `WORKER`"))
                .arg(arg!(<MIRROR> "指定 `MIRROR`"))
                .args(&common_flags)
                .arg_required_else_help(true)
        )
        .subcommand(
            Command::new("disable")
                .about("禁用一个同步任务")
                .arg(arg!(<WORKER> "指定 `WORKER`"))
                .arg(arg!(<MIRROR> "指定 `MIRROR`"))
                .args(&common_flags)
                .arg_required_else_help(true)
        )
        .subcommand(
            Command::new("restart")
                .about("重新开始一个同步任务")
                .arg(arg!(<WORKER> "指定 `WORKER`"))
                .arg(arg!(<MIRROR> "指定 `MIRROR`"))
                .args(&common_flags)
                .arg_required_else_help(true)
        )
        .subcommand(
            Command::new("reload")
                .about("通知 worker 重载配置文件")
                .arg(arg!(<WORKER> "指定 `WORKER`"))
                .args(&common_flags)
                .arg_required_else_help(true)
        )
        .subcommand(
            Command::new("ping")
                .arg(arg!(<WORKER> "指定 `WORKER`"))
                .arg(arg!(<MIRROR> "指定 `MIRROR`"))
                .args(&common_flags)
                .arg_required_else_help(true)
        )
        .arg_required_else_help(true)

}