use std::sync::Arc;

use flux_kcl_operator_crd::KclInstance;
use fluxcd_rs::Downloader;
use humantime::format_duration;
use kube::{runtime::controller::Action, Client, Discovery, Resource, ResourceExt};
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

    #[snafu(display("Failed to parse GVK: {}", source), visibility(pub))]
    FailedParseGvk {
        source: flux_kcl_operator_crd::Error,
    },

    #[snafu(display("Failed to publish event: {}", source))]
    PublishEvent { source: crate::event::Error },
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
    ///   will be created and deleted with this client.
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
    /// Update the subresources
    Update,
    /// This resource is in desired state and requires no actions to be taken
    NoOp,
}

/// Processes a KclInstance by downloading artifacts, rendering manifests, and applying changes
///
/// # Arguments
///
/// * `kcl_instance` - Reference to the KclInstance being processed
/// * `engine` - Engine used for downloading and rendering KCL artifacts
/// * `context` - Context data containing the downloader and other resources
///
/// # Returns
///
/// Returns Ok(()) if successful, or an Error if any step fails
async fn process_instance(
    kcl_instance: &Arc<KclInstance>,
    engine: &Engine,
    context: &ContextData,
) -> Result<()> {
    if kcl_instance.spec.suspend.unwrap_or(false) {
        info!("Instance suspended, skipping");
        return Ok(());
    }

    // Get or create default status for the instance
    let mut status = kcl_instance.status.clone().unwrap_or_default();

    // Get namespace for the instance
    let namespace = kcl_instance
        .namespace()
        .context(KclInstanceMissingNamespaceSnafu {
            name: kcl_instance.name_any(),
        })?;

    // Download KCL artifacts using the engine and downloader
    let artifacts_path = engine
        .download(kcl_instance.clone(), &context.downloader)
        .await
        .context(ArtefactsPathNotFoundSnafu)?;

    // Render the KCL manifests from the artifacts
    let manifests = engine
        .render(kcl_instance.clone(), &artifacts_path)
        .await
        .context(CannotRenderKclModuleSnafu)?;

    // Get current generation number for status tracking
    let current_generation = kcl_instance.metadata.generation.unwrap_or(0);

    // Process each manifest in the rendered output
    for dyno in multidoc_deserialize(manifests.as_str()).context(SplitYamlManifestsSnafu)? {
        let md = engine
            .apply(dyno.clone(), &namespace, &context.discovery)
            .await
            .context(EngineActionSnafu)?;

        // Add the applied manifest to the status inventory
        status
            .inventory
            .insert(md.try_into().context(FailedParseGvkSnafu)?);
    }

    // Update the instance status with changes
    engine
        .update_status(kcl_instance.clone(), status, current_generation)
        .await
        .context(EngineActionSnafu)?;
    Ok(())
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

            crate::event::publish_event(
                kcl_instance.clone(),
                client.clone(),
                "Reconcile".into(),
                "Creating".into(),
                Some(format!("Start creating resources {}", name)),
            )
            .await
            .context(PublishEventSnafu)?;

            // Add finalizer to prevent resource deletion until we're done with cleanup
            finalizer::add(client.clone(), name, &namespace)
                .await
                .context(AddFinalizerSnafu)?;
            info!("Added finalizer to resource {}", name);

            process_instance(&kcl_instance, engine, &context).await?;

            crate::event::publish_event(
                kcl_instance.clone(),
                client.clone(),
                "Reconcile".into(),
                "Ready".into(),
                Some(format!(
                    "Ready to apply all resorces. Next run in {}",
                    format_duration(kcl_instance.interval())
                )),
            )
            .await
            .context(PublishEventSnafu)?;

            Ok(Action::requeue(kcl_instance.interval()))
        }
        KclInstanceAction::Update => {
            info!("Update");

            process_instance(&kcl_instance, engine, &context).await?;

            Ok(Action::requeue(kcl_instance.interval()))
        }
        KclInstanceAction::Delete => {
            // Delete all subresources created in the `Create` phase

            if let Err(e) = engine
                .cleanup(kcl_instance.clone(), &context.discovery)
                .await
            {
                error!("Failed to cleanup: {}", e)
            }

            // Anyway delete finalizer, so we can delete the resource
            finalizer::delete(client.clone(), name, &namespace)
                .await
                .context(DeleteFinalizerSnafu)?;
            info!("Deleted finalizer from resource {}", name);

            crate::event::publish_event(
                kcl_instance.clone(),
                client.clone(),
                "Reconcile".into(),
                "Deleted".into(),
                Some("All resources deleted".to_string()),
            )
            .await
            .context(PublishEventSnafu)?;

            Ok(Action::await_change())
        }
        KclInstanceAction::NoOp => {
            info!("NoOp");
            Ok(Action::requeue(kcl_instance.interval()))
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
    context: Arc<ContextData>,
) -> Action {
    error!("Reconciliation error:\n{:?}.\n{:?}", error, kcl_instance);
    let client = context.client.clone();
    let interval = kcl_instance.interval();
    tokio::spawn(crate::event::publish_event(
        kcl_instance,
        client.clone(),
        "Reconcile".into(),
        "Error".into(),
        Some(error.to_string()),
    ));
    Action::requeue(interval)
}

fn determine_action(kcl_instance: &KclInstance) -> KclInstanceAction {
    if kcl_instance.meta().deletion_timestamp.is_some() {
        return KclInstanceAction::Delete;
    }

    if kcl_instance
        .meta()
        .finalizers
        .as_ref()
        .map_or(true, |finalizers| finalizers.is_empty())
    {
        return KclInstanceAction::Create;
    }

    if let Some(status) = kcl_instance.status.as_ref() {
        if status.observed_generation != kcl_instance.metadata.generation.unwrap_or(0) {
            return KclInstanceAction::Update;
        }
    }

    KclInstanceAction::NoOp
}
