use clap_complete::{generate_to, shells::Bash};
use std::{env, fs};
use std::io::Error;
use std::path::PathBuf;

include!("cli.rs");

fn main() -> Result<(), Error> {
    let mut out_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set"));
    out_dir.push("autocomplete");

    // 创建目录（如果不存在）
    if !out_dir.exists() {
        fs::create_dir_all(&out_dir)
            .expect("Failed to create autocomplete directory");
    }

    let mut cmd = build_cli();
    let path = generate_to(
        Bash,
        &mut cmd, // We need to specify what generator to use
        "rtsynctl",  // We need to specify the bin name manually
        out_dir,   // We need to specify where to write to
    ).expect("Failed to generate autocomplete");

    println!("脚本文件已经创建在 {path:?}");

    Ok(())
}