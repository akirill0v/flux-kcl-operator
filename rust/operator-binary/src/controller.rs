use std::{sync::Arc, time::Duration};

use flux_kcl_operator_crd::{Gvk, KclInstance, KclInstanceStatus};
use fluxcd_rs::Downloader;
use kube::{
    api::GroupVersionKind, core::gvk::ParseGroupVersionError, runtime::controller::Action, Client,
    Discovery, Resource, ResourceExt,
};
use snafu::{OptionExt, ResultExt, Snafu};
use strum::{EnumDiscriminants, IntoStaticStr};
use tracing::{error, info};

use crate::{
    engine::{self, Engine},
    finalizer,
    utils::multidoc_deserialize,
};

#[derive(Snafu, Debug, EnumDiscriminants)]
#[strum_discriminants(derive(IntoStaticStr))]
#[allow(clippy::enum_variant_names)]
pub enum Error {
    #[snafu(display("Failed to create kubernetes client: {}", source))]
    CannotCreateClient { source: kube::Error },

    #[snafu(display("Failed retrive namespace from resource: {}", name))]
    KclInstanceMissingNamespace { name: String },

    #[snafu(display("Failed to add finalizer: {}", source))]
    AddFinalizer { source: kube::Error },

    #[snafu(display("Failed to delete finalizer: {}", source))]
    DeleteFinalizer { source: kube::Error },

    #[snafu(display("Failed to download artifacts: {}", source))]
    ArtefactsPathNotFound { source: engine::Error },

    #[snafu(display("Failed to render kcl module: {}", source))]
    CannotRenderKclModule { source: engine::Error },

    #[snafu(display("Failed to split yaml manifests: {}", source))]
    SplitYamlManifests { source: anyhow::Error },

    #[snafu(display("Failed with engine action: {}", source))]
    EngineAction { source: engine::Error },

    #[snafu(display("Failed to get object key: {}", key))]
    MissingObjectKey { key: String },

    #[snafu(display("Failed to parse GVK: {}", source))]
    FaieldToParseGvk { source: ParseGroupVersionError },
}

type Result<T, E = Error> = std::result::Result<T, E>;

/// Context injected with each `reconcile` and `on_error` method invocation.
pub struct ContextData {
    /// Kubernetes client to make Kubernetes API requests with. Required for K8S resource management.
    client: Client,

    downloader: Downloader,
    engine: Engine,
    discovery: Discovery,
}

impl ContextData {
    /// Constructs a new instance of ContextData.
    ///
    /// # Arguments:
    /// - `client`: A Kubernetes client to make Kubernetes REST API requests with. Resources
    /// will be created and deleted with this client.
    pub fn new(
        client: Client,
        downloader: Downloader,
        engine: Engine,
        discovery: Discovery,
    ) -> Self {
        ContextData {
            client,
            downloader,
            engine,
            discovery,
        }
    }
}

/// Action to be taken upon an `KclInstance` resource during reconciliation
#[derive(Debug)]
enum KclInstanceAction {
    /// Create the subresources
    Create,
    /// Delete all subresources created in the `Create` phase
    Delete,
    /// This resource is in desired state and requires no actions to be taken
    NoOp,
}

pub async fn reconcile(
    kcl_instance: Arc<KclInstance>,
    context: Arc<ContextData>,
) -> Result<Action, Error> {
    let client = context.client.clone();
    let engine = &context.engine;
    let name = &kcl_instance.name_any();

    let namespace = kcl_instance
        .namespace()
        .context(KclInstanceMissingNamespaceSnafu { name })?;

    match determine_action(&kcl_instance) {
        KclInstanceAction::Create => {
            info!("KclInstance {} is being created", name);

            let mut status = KclInstanceStatus {
                ..Default::default()
            };

            // Add finalizer to prevent resource deletion until we're done with cleanup
            finalizer::add(client.clone(), name, &namespace)
                .await
                .context(AddFinalizerSnafu)?;
            info!("Added finalizer to resource {}", name);

            // Download KCL artifacts using the engine and downloader
            let artifacts_path = engine
                .download(kcl_instance.clone(), &context.downloader)
                .await
                .context(ArtefactsPathNotFoundSnafu)?;

            let manifests = engine
                .render(kcl_instance.clone(), &artifacts_path)
                .await
                .context(CannotRenderKclModuleSnafu)?;

            for dyno in multidoc_deserialize(manifests.as_str()).context(SplitYamlManifestsSnafu)? {
                let md = engine
                    .apply(dyno.clone(), &namespace, &context.discovery)
                    .await
                    .context(EngineActionSnafu)?;

                let type_meta = md.types.clone().context(MissingObjectKeySnafu {
                    key: "metadata/typeMeta",
                })?;

                let g_gvk =
                    GroupVersionKind::try_from(&type_meta).context(FaieldToParseGvkSnafu)?;
                let gvk = Gvk {
                    name: md.name_any(),
                    group: g_gvk.group,
                    version: g_gvk.version,
                    kind: g_gvk.kind,
                    namespace: dyno.namespace(),
                };
                status.inventory.insert(gvk);
            }

            engine
                .update_status(kcl_instance.clone(), status)
                .await
                .context(EngineActionSnafu)?;

            Ok(Action::requeue(Duration::from_secs(10)))
        }
        KclInstanceAction::Delete => {
            // Delete all subresources created in the `Create` phase

            if let Err(e) = engine.cleanup(kcl_instance, &context.discovery).await {
                error!("Failed to cleanup: {}", e)
            }

            finalizer::delete(client, name, &namespace)
                .await
                .context(DeleteFinalizerSnafu)?;
            info!("Deleted finalizer from resource {}", name);

            Ok(Action::await_change())
        }
        KclInstanceAction::NoOp => {
            info!("NoOp");
            Ok(Action::requeue(Duration::from_secs(10)))
        } // TODO: Change interval from KclInstance
    }
}

/// Actions to be taken when a reconciliation fails - for whatever reason.
/// Prints out the error to `stderr` and requeues the resource for another reconciliation after
/// five seconds.
///
/// # Arguments
/// - `kcl_instance`: The erroneous resource.
/// - `error`: A reference to the `kube::Error` that occurred during reconciliation.
/// - `_context`: Unused argument. Context Data "injected" automatically by kube-rs.
pub fn on_error(
    kcl_instance: Arc<KclInstance>,
    error: &Error,
    _context: Arc<ContextData>,
) -> Action {
    eprintln!("Reconciliation error:\n{:?}.\n{:?}", error, kcl_instance);
    Action::requeue(Duration::from_secs(5))
}

fn determine_action(kcl_instance: &KclInstance) -> KclInstanceAction {
    if kcl_instance.meta().deletion_timestamp.is_some() {
        KclInstanceAction::Delete
    } else if kcl_instance
        .meta()
        .finalizers
        .as_ref()
        .map_or(true, |finalizers| finalizers.is_empty())
    {
        KclInstanceAction::Create
    } else {
        KclInstanceAction::NoOp
    }
}
