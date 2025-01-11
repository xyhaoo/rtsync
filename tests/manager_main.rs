use manager;

#[rocket::main]
async fn main() {
    let c = clap::Command::new("manager").get_matches();
    let cfg = manager::config::load_config(Some("tests/manager.conf".to_string()), &c).unwrap();
    let m = manager::server::get_rtsync_manager(&cfg).unwrap();
    m.run().await;
}