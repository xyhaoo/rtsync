use std::fs::File;
use std::io::Write;
use std::process::exit;
use clap;
use clap::{arg, Arg, ArgAction, ArgMatches, Command, Subcommand};
use log::{error, info};
use anyhow::Result;
use tokio::signal::unix::{signal, SignalKind};
use pprof::ProfilerGuard;
use pprof::protos::Message;
use scopeguard::defer;
use manager;
use worker;
use internal as rtsync;
mod cli;
mod build;

async fn start_manager(c: &ArgMatches) -> Result<()> {
    rtsync::logger::init_logger(c.get_flag("verbose"), 
                                c.get_flag("debug"), 
                                c.get_flag("with-systemd"));
    
    match manager::config::load_config(c.get_one::<String>("FILE").cloned(), c){
        Err(e) => { 
            error!("Error loading config: {}", e);
            exit(1);
        },
        Ok(config) => {
            match manager::server::get_rtsync_manager(&config){
                Err(e) => {
                    error!("Error initializing RT sync worker.");
                    exit(1);
                },
                Ok(manager) => {
                    info!("Run rtsync manager server.");
                    tokio::spawn(async move {
                        manager.run().await;
                    });
                }
            }
        }
    }
    Ok(())
}

async fn start_worker(c: &ArgMatches) -> Result<()> {
    rtsync::logger::init_logger(c.get_flag("verbose"), 
                                c.get_flag("debug"), 
                                c.get_flag("with-systemd"));
    let config_path = c.get_one::<String>("config").cloned().unwrap();
    match worker::config::load_config(&config_path){
        Err(e) => {
            error!("Error loading config: {}", e);
            exit(1);
        },
        Ok(config) => {
            match worker::worker::Worker::new(config).await {
                None => {
                    error!("Error initializing RT sync worker.");
                    exit(1);
                },
                Some(w) => {
                    if let Some(prof_path) = c.get_one::<String>("prof-path"){
                        if std::path::Path::new(prof_path).is_dir(){
                            let guard = ProfilerGuard::new(100).unwrap();
                            defer!{
                                if let Ok(report) = guard.report().build() {
                                    let mut file = File::create(format!("{prof_path}/profile.pb")).unwrap();
                                    let profile = report.pprof().unwrap();
                                    let mut content = Vec::new();
                                    profile.encode(&mut content).unwrap();
                                    file.write_all(&content).unwrap();
                                };
                            }
                        }else { 
                            error!("Invalid profiling path: {}", prof_path);
                            exit(1);
                        }
                    }
                    let mut w_clone = w.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        let mut sighup = signal(SignalKind::hangup()).unwrap();
                        let mut sigint = signal(SignalKind::interrupt()).unwrap();
                        let mut sigterm = signal(SignalKind::terminate()).unwrap();
                        loop{
                            tokio::select! {
                                _ = sighup.recv() => {
                                    info!("Received reload signal");
                                    match worker::config::load_config(&config_path){
                                        Err(e) => {
                                            error!("Error loading config: {}", e);
                                        },
                                        Ok(new_cfg) => {
                                            w_clone.reload_mirror_config(new_cfg.mirrors).await;
                                        }
                                    }
                                },
                                _ = sigint.recv() => {
                                    w_clone.halt().await;
                                },
                                _ = sigterm.recv() => {
                                    w_clone.halt().await;
                                },            
                            }
                        }
                    });

                    info!("Run rtsync worker.");
                    w.run().await;
                }
            }
        }
    }
    Ok(())
}


#[rocket::main]
async fn main() -> Result<()> {
    let matches = cli::build_cli()
        .get_matches();

    match matches.subcommand() {
        Some(("manager", sub_matches)) => {
            if sub_matches.args_present() {
                start_manager(sub_matches).await?;
            }
        },
        Some(("worker", sub_matches)) => {
            if sub_matches.args_present() {
                start_worker(sub_matches).await?;
            }
        },
        _ => unreachable!()
    }
    
    Ok(())
}