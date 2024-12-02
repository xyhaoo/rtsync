#[cfg(test)]
mod tests {
    use tokio;
    use std::io::Write;
    use std::{io, result};
    use std::collections::HashMap;
    use std::sync::Arc;
    use internal::util::{get_tls_config, create_http_client, get_json};

    #[tokio::test]
    async fn main(){
        let client = create_http_client(Some("/Users/xiaoyh/Documents/Code/课堂记录/Rust/rtsync/tests/rootCA.crt")).await.unwrap();
        let client = Arc::new(Some(client));
        let msg: HashMap<String, String> = HashMap::new();
        let resp = get_json::<HashMap<String, String>>("https://localhost:5002/", client).await.unwrap();
    }
}
