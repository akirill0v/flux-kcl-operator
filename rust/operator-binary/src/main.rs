mod controller;
mod fetcher;

use std::sync::Arc;

use clap::{crate_description, crate_version, Parser};
use fetcher::Fetcher;
use flux_kcl_operator_crd::KclInstance;
use futures::stream::StreamExt;
use reqwest_middleware::ClientBuilder;
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use stackable_operator::{
    cli::{Command, ProductOperatorRun},
    k8s_openapi::api::{
        apps::v1::{Deployment, StatefulSet},
        core::v1::{ConfigMap, Service},
    },
    kube::{
        core::DeserializeGuard,
        runtime::{watcher, Controller},
    },
    logging::controller::report_controller_reconciled,
    CustomResourceExt,
};

use crate::controller::KCL_INSTANCE_NAME;

const OPERATOR_NAME: &str = "instance.kcl.evrone.com";

mod built_info {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

#[derive(Parser)]
#[clap(about, author)]
struct Opts {
    #[arg(long, env = "KCL_HTTP_RETRY")]
    http_retry: Option<u32>,

    #[arg(long, env = "SOURCE_HOST")]
    source_host: Option<String>,

    #[arg(long, env = "KCL_STORAGE_DIR")]
    storage_dir: Option<std::path::PathBuf>,

    #[clap(subcommand)]
    cmd: Command,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opts = Opts::parse();
    match opts.cmd {
        Command::Crd => KclInstance::print_yaml_schema(built_info::PKG_VERSION)?,
        Command::Run(ProductOperatorRun {
            product_config,
            watch_namespace,
            tracing_target,
            cluster_info_opts,
        }) => {
            stackable_operator::logging::initialize_logging(
                "KCL_INSTANCE_OPERATOR_LOG",
                "kcl-instance-operator",
                tracing_target,
            );
            stackable_operator::utils::print_startup_string(
                crate_description!(),
                crate_version!(),
                built_info::GIT_VERSION,
                built_info::TARGET,
                built_info::BUILT_TIME_UTC,
                built_info::RUSTC_VERSION,
            );

            let product_config = product_config.load(&[
                "deploy/config-spec/properties.yaml",
                "/etc/stackable/nifi-operator/config-spec/properties.yaml",
            ])?;

            let storage_dir = opts.storage_dir.unwrap_or_else(|| {
                std::env::temp_dir()
                    .join("kcl-instance-operator")
                    .join(OPERATOR_NAME)
            });

            tracing::info!("Store artefacts in {:?}", storage_dir);
            if !storage_dir.exists() {
                tracing::info!("Create storage dir: {:?}", storage_dir);
                std::fs::create_dir_all(&storage_dir)?;
            }

            let client = stackable_operator::client::initialize_operator(
                Some(OPERATOR_NAME.to_string()),
                &cluster_info_opts,
            )
            .await?;

            let retry_policy =
                ExponentialBackoff::builder().build_with_max_retries(opts.http_retry.unwrap_or(1));
            let http_client = ClientBuilder::new(reqwest::Client::new())
                .with(RetryTransientMiddleware::new_with_policy(retry_policy))
                .build();

            let kcl_instance_controller = Controller::new(
                watch_namespace.get_api::<DeserializeGuard<KclInstance>>(&client),
                watcher::Config::default(),
            );

            let kcl_instance_store = kcl_instance_controller.store();

            kcl_instance_controller
                .owns(
                    watch_namespace.get_api::<Service>(&client),
                    watcher::Config::default(),
                )
                .owns(
                    watch_namespace.get_api::<Deployment>(&client),
                    watcher::Config::default(),
                )
                .owns(
                    watch_namespace.get_api::<StatefulSet>(&client),
                    watcher::Config::default(),
                )
                .owns(
                    watch_namespace.get_api::<ConfigMap>(&client),
                    watcher::Config::default(),
                )
                .shutdown_on_signal()
                // .watches(
                //     client.get_api::<DeserializeGuard<AuthenticationClass>>(&()),
                //     watcher::Config::default(),
                //     move |_| {
                //         kcl_instance_store
                //             .state()
                //             .into_iter()
                //             .map(|kcl_instance| ObjectRef::from_obj(&*kcl_instance))
                //     },
                // )
                .run(
                    controller::reconcile,
                    controller::error_policy,
                    Arc::new(controller::Ctx {
                        client: client.clone(),
                        fetcher: Fetcher::new(http_client, opts.source_host),
                        storage_dir,
                        product_config,
                    }),
                )
                .map(|res| {
                    report_controller_reconciled(
                        &client,
                        &format!("{KCL_INSTANCE_NAME}.{OPERATOR_NAME}"),
                        &res,
                    )
                })
                .collect::<()>()
                .await;
        }
    }

    Ok(())
}
