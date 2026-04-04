use clap::Parser;
use k8s_crd_groupers::{GroupBy, GroupedCrds};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "crd-grouper", about = "List and group Kubernetes custom resources")]
struct Cli {
    /// How to group the results
    #[arg(short, long, value_enum, default_value_t = GroupBy::Group)]
    group_by: GroupBy,

    /// Output as JSON instead of human-readable
    #[arg(long)]
    json: bool,
}

fn print_human(grouped: &GroupedCrds) {
    println!(
        "CRDs: {}  |  Instances: {}  |  Grouped by: {}",
        grouped.total_crds, grouped.total_instances, grouped.group_by
    );
    println!("{}", "=".repeat(60));

    for (key, instances) in &grouped.groups {
        println!("\n[{}] ({} instances)", key, instances.len());
        for inst in instances {
            let ns = inst
                .namespace
                .as_deref()
                .unwrap_or("<cluster>");
            println!("  - {}/{} ({})", ns, inst.name, inst.kind);
        }
    }
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let client = k8s_crd_groupers::create_client().await?;
    let grouped = k8s_crd_groupers::discover_and_group(client, cli.group_by).await?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&grouped)?);
    } else {
        print_human(&grouped);
    }

    Ok(())
}
