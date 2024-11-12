use std::{env, sync::Arc};

use clap::{Parser, Subcommand};
use flux_kcl_operator::controller::{self, ContextData};
use flux_kcl_operator_crd::KclInstance;
use futures::stream::StreamExt;
use kube::{
    runtime::{watcher::Config, Controller},
    Api, Client, CustomResourceExt, Discovery,
};
use reqwest_middleware::ClientBuilder;
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Layer, Registry};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[arg(long, env = "KCL_HTTP_RETRY")]
    http_retry: Option<u32>,

    #[arg(long, env = "SOURCE_HOST")]
    source_host: Option<String>,

    #[arg(long, env = "KCL_STORAGE_DIR")]
    storage_dir: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Render the CRD YAML
    Crd,
    /// Run the operator
    Run,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_logger()?;

    let cli = Cli::parse();

    match cli.command {
        Commands::Crd => {
            println!("{}", serde_yaml::to_string(&KclInstance::crd())?);
            Ok(())
        }
        Commands::Run => {
            let client = Client::try_default().await?;

            let discovery = Discovery::new(client.clone())
                .run()
                .await
                .expect("Failed to create discovery client");

            let context: Arc<ContextData> = init_context(client.clone(), cli, discovery);

            let api_kcl_instance: Api<KclInstance> = Api::all(client.clone());

            // Run the operator's controller in a loop, processing each instance of the custom resource
            Controller::new(api_kcl_instance.clone(), Config::default())
                .run(controller::reconcile, controller::on_error, context)
                .for_each(|reconciliation_result| async move {
                    match reconciliation_result {
                        Ok(resource) => {
                            info!("Reconciliation successful. Resource: {:?}", resource)
                        }
                        Err(err) => error!("Reconciliation error: {:?}", err),
                    }
                })
                .await;
            Ok(())
        }
    }
}

/// Initializes the context data for the operator.
///
/// # Arguments
/// * `client` - The Kubernetes client
/// * `cli` - The command line arguments
///
/// # Returns
/// A new `Arc<ContextData>` containing the initialized context
fn init_context(client: kube::Client, cli: Cli, discovery: Discovery) -> Arc<ContextData> {
    let retry_policy =
        ExponentialBackoff::builder().build_with_max_retries(cli.http_retry.unwrap_or(1));
    let http_client = ClientBuilder::new(reqwest::Client::new())
        .with(RetryTransientMiddleware::new_with_policy(retry_policy))
        .build();

    let downloader =
        fluxcd_rs::downloader::Downloader::new(http_client, cli.source_host, cli.storage_dir);
    let engine = flux_kcl_operator::engine::Engine::new(client.clone());

    Arc::new(ContextData::new(client, downloader, engine, discovery))
}

/// Initializes a logger with environment filters and formatting.
///
/// # Returns
/// * `Ok(())` if logger initialization was successful
/// * `Err` if there was an error setting up the logger
fn init_logger() -> Result<(), Box<dyn std::error::Error>> {
    let filter_layer = match env::var("RUST_LOG") {
        Ok(e) => EnvFilter::new(e),
        _ => EnvFilter::try_from("info")?,
    };

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_line_number(true);

    let subscriber = Registry::default().with(filter_layer).with(fmt_layer);

    tracing::subscriber::set_global_default(subscriber)?;
    Ok(())
}
