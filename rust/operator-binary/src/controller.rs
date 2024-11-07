use std::{sync::Arc, time::Duration};

use flux_kcl_operator_crd::KclInstance;
use fluxcd_rs::Downloader;
use kube::{runtime::controller::Action, Client, Resource, ResourceExt};
use snafu::{OptionExt, ResultExt, Snafu};
use strum::{EnumDiscriminants, IntoStaticStr};
use tracing::info;

use crate::{engine::Engine, finalizer};

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
}

type Result<T, E = Error> = std::result::Result<T, E>;

/// Context injected with each `reconcile` and `on_error` method invocation.
pub struct ContextData {
    /// Kubernetes client to make Kubernetes API requests with. Required for K8S resource management.
    client: Client,

    downloader: Downloader,
    engine: Engine,
}

impl ContextData {
    /// Constructs a new instance of ContextData.
    ///
    /// # Arguments:
    /// - `client`: A Kubernetes client to make Kubernetes REST API requests with. Resources
    /// will be created and deleted with this client.
    pub fn new(client: Client, downloader: Downloader, engine: Engine) -> Self {
        ContextData {
            client,
            downloader,
            engine,
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
            finalizer::add(client, name, &namespace)
                .await
                .context(AddFinalizerSnafu)?;
            info!("Added finalizer to resource {}", name);
            let artifact = engine
                .download(kcl_instance.clone(), &context.downloader)
                .await;
            Ok(Action::requeue(Duration::from_secs(10)))
        }
        KclInstanceAction::Delete => {
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
