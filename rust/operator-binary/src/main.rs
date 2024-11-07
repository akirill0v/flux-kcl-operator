use std::{env, sync::Arc};

use clap::{Parser, Subcommand};
use flux_kcl_operator::controller::{self, ContextData};
use flux_kcl_operator_crd::KclInstance;
use futures::stream::StreamExt;
use kube::{
    runtime::{watcher::Config, Controller},
    Api, Client, CustomResourceExt,
};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Layer, Registry};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
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

            let api_kcl_instance: Api<KclInstance> = Api::all(client.clone());
            let context: Arc<ContextData> = Arc::new(ContextData::new(client.clone()));

            Controller::new(api_kcl_instance.clone(), Config::default())
                .run(controller::reconcile, controller::on_error, context)
                .for_each(|reconciliation_result| async move {
                    match reconciliation_result {
                        Ok(resource) => {
                            info!("Reconciliation successful. Resource: {:?}", resource);
                        }
                        Err(reconciliation_err) => {
                            error!("Reconciliation error: {:?}", reconciliation_err)
                        }
                    }
                })
                .await;
            Ok(())
        }
    }
}

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
