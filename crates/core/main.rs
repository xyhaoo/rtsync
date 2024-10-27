use clap::Parser;


mod cli;

fn main() {
    let cli = cli::Cli::parse();

    // 使用解析的参数
    println!("Input: {}", cli.input);
    println!("Output: {}", cli.output);
    println!("Verbose: {}", cli.verbose);
}
