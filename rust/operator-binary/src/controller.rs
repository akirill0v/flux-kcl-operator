use std::{path::PathBuf, sync::Arc, time::Duration};

use flux_kcl_operator_crd::{KclInstance, APP_NAME};
use fluxcd_rs::{FluxSourceArtefact, GitRepository, GitRepositoryStatusArtifact, OCIRepository};
use product_config::ProductConfigManager;
use reqwest_middleware::ClientWithMiddleware;
use snafu::{OptionExt, ResultExt, Snafu};
use stackable_operator::{
    builder::{configmap::ConfigMapBuilder, meta::ObjectMetaBuilder},
    client::Client,
    cluster_resources::{ClusterResourceApplyStrategy, ClusterResources},
    k8s_openapi::api::core::v1::ConfigMap,
    kube::{
        core::{error_boundary, DeserializeGuard},
        runtime::{
            controller::Action,
            reflector::{Lookup, ObjectRef},
        },
        Resource, ResourceExt,
    },
    kvp::ObjectLabels,
    logging::controller::ReconcilerError,
};
use strum::{EnumDiscriminants, IntoStaticStr};
use tracing::info;

use crate::{fetcher::Fetcher, OPERATOR_NAME};

pub const KCL_INSTANCE_NAME: &str = "kcl-instance";
pub const KCL_INSTANCE_CONTROLLER_NAME: &str = "kcl-instance-controller";
pub const KCL_PROPERTIES_KEY: &str = "properties";
pub const KCL_MANIFESTS_KEY: &str = "manifests";

pub struct Ctx {
    pub client: Client,
    pub fetcher: Fetcher,
    pub product_config: ProductConfigManager,
    pub storage_dir: PathBuf,
}

#[derive(Snafu, Debug, EnumDiscriminants)]
#[strum_discriminants(derive(IntoStaticStr))]
#[allow(clippy::enum_variant_names)]
pub enum Error {
    #[snafu(display("KclInstance object is invalid"))]
    InvalidKclInstance {
        source: error_boundary::InvalidObject,
    },

    #[snafu(display("object defines no name"))]
    ObjectHasNoName,

    #[snafu(display("object defines no spec"))]
    ObjectHasNoSpec,

    #[snafu(display("object defines no kind"))]
    ObjectHasNoKind,

    #[snafu(display("object defines no namespace"))]
    ObjectHasNoNamespace,

    #[snafu(display("object defines no status"))]
    ObjectHasNoStatus,

    #[snafu(display("object defines no artifact"))]
    ObjectHasNoArtefact,

    #[snafu(display("failed to create cluster resources"))]
    CreateClusterResources {
        source: stackable_operator::cluster_resources::Error,
    },

    #[snafu(display("failed to delete orphaned resources"))]
    DeleteOrphanedResources {
        source: stackable_operator::cluster_resources::Error,
    },

    #[snafu(display("failed to build metadata"))]
    MetadataBuild {
        source: stackable_operator::builder::meta::Error,
    },

    #[snafu(display("failed to build configmap"))]
    BuildConfigMap {
        source: stackable_operator::builder::configmap::Error,
    },

    #[snafu(display("failed to update status"))]
    StatusUpdate {
        source: stackable_operator::client::Error,
    },

    #[snafu(display("failed to apply configmap"))]
    ApplyConfigMap {
        source: stackable_operator::cluster_resources::Error,
    },

    #[snafu(display("failed to find kubernetes object"))]
    ObjectHasNotFound {
        source: stackable_operator::client::Error,
    },

    #[snafu(display("failed to download source artifact: {}", source))]
    DownloadSourceArtifact {
        source: crate::fetcher::FetcherError,
    },
}

type Result<T, E = Error> = std::result::Result<T, E>;

impl ReconcilerError for Error {
    fn category(&self) -> &'static str {
        ErrorDiscriminants::from(self).into()
    }
}

pub async fn reconcile(
    kcl_instance: Arc<DeserializeGuard<KclInstance>>,
    ctx: Arc<Ctx>,
) -> Result<Action> {
    tracing::info!("Starting reconcile");

    let kcl_instance = kcl_instance
        .0
        .as_ref()
        .map_err(error_boundary::InvalidObject::clone)
        .context(InvalidKclInstanceSnafu)?;

    let source = &kcl_instance.spec.source;

    let client = &ctx.client;

    let namespace = &kcl_instance
        .metadata
        .namespace
        .clone()
        .with_context(|| ObjectHasNoNamespaceSnafu {})?;

    let mut cluster_resources = ClusterResources::new(
        APP_NAME,
        OPERATOR_NAME,
        KCL_INSTANCE_CONTROLLER_NAME,
        &kcl_instance.object_ref(&()),
        ClusterResourceApplyStrategy::from(&kcl_instance.spec.cluster_operation),
    )
    .context(CreateClusterResourcesSnafu)?;

    // Fetch FluxCD srouce reference artifact
    // Find the source reference in the kcl instance
    let source_name = source.name.as_ref().context(ObjectHasNoNameSnafu)?;
    let source_namespace = source
        .namespace
        .as_ref()
        .or(kcl_instance.metadata.namespace.as_ref())
        .context(ObjectHasNoNamespaceSnafu)?;

    let artefact: Option<FluxSourceArtefact> = match source.kind.as_deref() {
        Some("GitRepository") => Some(FluxSourceArtefact::Git(
            client
                .get::<GitRepository>(source_name, source_namespace)
                .await
                .context(ObjectHasNotFoundSnafu)?
                .status
                .context(ObjectHasNoStatusSnafu)?
                .artifact
                .context(ObjectHasNoArtefactSnafu)?,
        )),
        Some("OciRepository") => Some(FluxSourceArtefact::Oci(
            client
                .get::<OCIRepository>(source_name, source_namespace)
                .await
                .context(ObjectHasNotFoundSnafu)?
                .status
                .context(ObjectHasNoStatusSnafu)?
                .artifact
                .context(ObjectHasNoArtefactSnafu)?,
        )),
        _ => None,
    };

    if let Some(artefact) = artefact {
        tracing::info!("Found source reference");
        let work_dir = ctx
            .fetcher
            .download(
                &artefact.url(),
                source_name,
                ctx.storage_dir.join(source_namespace),
            )
            .await
            .context(DownloadSourceArtifactSnafu)?;

        let manifests = process_source(&work_dir, client).await?;
    } else {
        tracing::warn!(
            "No source reference found, retry in {:?}",
            &kcl_instance.interval()
        );
        return Ok(Action::requeue(kcl_instance.interval()));
    }

    // let kcl_instance_config = build_kcl_instance_config(kcl_instance, &ctx.product_config).await?;

    // dbg!(&kcl_instance_config);

    // cluster_resources
    //     .add(client, kcl_instance_config)
    //     .await
    //     .context(ApplyConfigMapSnafu)?;

    // cluster_resources
    //     .delete_orphaned_resources(client)
    //     .await
    //     .context(DeleteOrphanedResourcesSnafu)?;

    Ok(Action::requeue(kcl_instance.interval()))
}

async fn process_source(work_dir: &PathBuf, _client: &Client) -> Result<String> {
    info!("Processing source in {}", work_dir.display());

    Ok(String::from("todo"))
}

async fn build_kcl_instance_config(
    kcl_instance: &KclInstance,
    product_config: &ProductConfigManager,
) -> Result<ConfigMap> {
    tracing::debug!("Building config map for kcl instance");

    let mut cm_builder = ConfigMapBuilder::new();

    cm_builder
        .metadata(
            ObjectMetaBuilder::new()
                .name_and_namespace(kcl_instance)
                .ownerreference_from_resource(kcl_instance, None, Some(true))
                .context(MetadataBuildSnafu)?
                .with_recommended_labels(build_recommended_labels(kcl_instance, "0.0.0"))
                .context(MetadataBuildSnafu)?
                .build(),
        )
        .add_data("values", "test");

    cm_builder.build().context(BuildConfigMapSnafu)
}

pub fn error_policy(
    obj: Arc<DeserializeGuard<KclInstance>>,
    error: &Error,
    _ctx: Arc<Ctx>,
) -> Action {
    match error {
        // root object is invalid, will be requeued when modified anyway
        Error::InvalidKclInstance { .. } => Action::await_change(),
        _ => {
            let interval = obj
                .0
                .clone()
                .map(|o| o.interval())
                .unwrap_or(Duration::from_secs(10));
            Action::requeue(interval)
        }
    }
}

pub fn build_recommended_labels<'a>(
    owner: &'a KclInstance,
    app_version: &'a str,
) -> ObjectLabels<'a, KclInstance> {
    ObjectLabels {
        owner,
        app_name: APP_NAME,
        app_version,
        operator_name: OPERATOR_NAME,
        controller_name: KCL_INSTANCE_CONTROLLER_NAME,
        role: "rr",
        role_group: "rg",
    }
}
