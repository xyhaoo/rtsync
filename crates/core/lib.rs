
use log::{info, LevelFilter};
use env_logger;

pub fn init() {
    env_logger::builder()
        .filter_level(LevelFilter::Info)
        .init();
}