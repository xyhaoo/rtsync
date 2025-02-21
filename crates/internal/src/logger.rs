use fern::colors::{Color, ColoredLevelConfig};
use chrono::Local;
use log::LevelFilter;
// init_logger 初始化log格式和级别

pub fn init_logger(verbose: bool, debug: bool, with_systemd: bool) {
    // 配置颜色
    let colors = ColoredLevelConfig::new()
        .error(Color::Red)
        .warn(Color::Yellow)
        .info(Color::Green)
        .debug(Color::Blue)
        .trace(Color::Magenta);

    // 构建日志配置
    let mut base_config = fern::Dispatch::new();

    // 设置日志格式
    base_config = base_config.format(move |out, message, record| {
        if with_systemd {
            // Systemd格式，不包含时间和文件信息
            out.finish(format_args!(
                "[{:>6}] {}",
                colors.color(record.level()),
                message
            ));
        } else if debug {
            // Debug模式下的格式，包含时间和文件信息
            out.finish(format_args!(
                "[{}][{:>6}][{}] {}",
                Local::now().format("%y-%m-%d %H:%M:%S"),
                colors.color(record.level()),
                record.target(),
                message
            ));
        } else {
            // 非debug模式的格式，仅包含时间和日志级别
            out.finish(format_args!(
                "[{}][{:>6}] {}",
                Local::now().format("%y-%m-%d %H:%M:%S"),
                colors.color(record.level()),
                message
            ));
        }
    });

    // 设置日志级别
    base_config = base_config.level(if debug {
        LevelFilter::Debug
    } else if verbose {
        LevelFilter::Info
    } else {
        LevelFilter::Warn
    });

    // 将配置应用于标准输出
    base_config = base_config.chain(std::io::stdout());

    // 初始化日志系统
    base_config.apply().expect("Failed to initialize logger");
}


#[cfg(test)]
mod tests {
    use log::{info, debug, warn, error};
    use super::*;
    
    #[test]
    fn test_all_false(){
        init_logger(false, false, false);

        info!("This is an info message.");
        debug!("This is a debug message.");
        warn!("This is a warning message.");
        error!("This is an error message.");
    }
    #[test]
    fn test_verbose(){
        init_logger(true, false, false);

        info!("This is an info message.");
        debug!("This is a debug message.");
        warn!("This is a warning message.");
        error!("This is an error message.");  
    }
    #[test]
    fn test_debug(){
        init_logger(false, true, false);

        info!("This is an info message.");
        debug!("This is a debug message.");
        warn!("This is a warning message.");
        error!("This is an error message.");
    }
    #[test]
    fn test_systemd(){
        init_logger(false, false, true);

        info!("This is an info message.");
        debug!("This is a debug message.");
        warn!("This is a warning message.");
        error!("This is an error message.");
    }
    #[test]
    fn test_v_d(){
        init_logger(true, true, false);

        info!("This is an info message.");
        debug!("This is a debug message.");
        warn!("This is a warning message.");
        error!("This is an error message.");
    }
    #[test]
    fn test_v_s(){
        init_logger(true, false, true);

        info!("This is an info message.");
        debug!("This is a debug message.");
        warn!("This is a warning message.");
        error!("This is an error message.");
    }
    #[test]
    fn test_d_s(){
        init_logger(false, true, true);

        info!("This is an info message.");
        debug!("This is a debug message.");
        warn!("This is a warning message.");
        error!("This is an error message.");
    }
    #[test]
    fn test_all_true(){
        init_logger(true, true, true);

        info!("This is an info message.");
        debug!("This is a debug message.");
        warn!("This is a warning message.");
        error!("This is an error message.");
    }
}