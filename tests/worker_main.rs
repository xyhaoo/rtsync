use worker;

#[tokio::main]
async fn main() {
    let cfg = worker::config::load_config("tests/worker.conf").unwrap();
    let m = worker::worker::Worker::new(cfg).await.unwrap();
    m.run().await
}