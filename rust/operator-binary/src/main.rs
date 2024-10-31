mod controller;
mod kcl;

use std::sync::Arc;

use clap::{crate_description, crate_version, Parser};
use flux_kcl_operator_crd::KclInstance;
use futures::stream::StreamExt;
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

            let client = stackable_operator::client::initialize_operator(
                Some(OPERATOR_NAME.to_string()),
                &cluster_info_opts,
            )
            .await?;

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
