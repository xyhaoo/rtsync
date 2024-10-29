mod config;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "manager", version = "1.0", author = "Your Name", about = "An example CLI application")]
pub struct Cli{
    #[arg(short = 'c', long)]
    pub config: Option<String>,
    
    #[arg(short, long)]
    pub addr: Option<String>,
    
    #[arg(short, long)]
    pub port: Option<u32>,  //端口号的正确值在1-65535之间，

    #[arg(long)]
    pub cert: Option<String>,
    
    #[arg(short, long)]
    pub key: Option<String>,
    
    #[arg(short, long = "status-file")]
    pub status_file: Option<String>,

    #[arg(long = "db-file")]
    pub db_file: Option<String>,

    #[arg(long = "db-type")]
    pub db_type: Option<String>,
}
