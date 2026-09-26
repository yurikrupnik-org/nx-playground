//! Print status + connection info for a DevEnvironment claim.
//! Usage: cargo run -p devenv-sdk --example status -- [name] [namespace]

use devenv_sdk::DevEnvClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let name = args.next().unwrap_or_else(|| "demo".into());
    let namespace = args.next().unwrap_or_else(|| "default".into());

    let client = DevEnvClient::new().await?;
    let status = client.get(&name, &namespace).await?;
    println!("ready: {}", status.ready);
    println!("environment: {:#?}", status.environment);

    if status.ready {
        let conn = client.connection(&name, &namespace).await?;
        if let Some(pg) = &conn.postgres {
            println!(
                "postgres: {}@{}:{}/{} (password: ********)",
                pg.username, pg.host, pg.port, pg.dbname
            );
        }
        println!("redis: {:?}", conn.redis_host);
        println!("nats:  {:?}", conn.nats_url);
    } else {
        eprintln!("environment not ready yet");
    }
    Ok(())
}
