use clap::{Parser};

#[derive(Parser)]
#[command(name = "example", version = "1.0", about = "An example CLI application")]
pub struct Cli {
    /// The input file
    #[arg(short = 'i', long = "input")]
    pub input: String,

    /// The output file
    #[arg(short = 'o', long = "output")]
    pub output: String,

    /// Verbose mode
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,
}

