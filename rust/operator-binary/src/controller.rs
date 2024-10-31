use std::{sync::Arc, time::Duration};

use flux_kcl_operator_crd::{KclInstance, APP_NAME};
use product_config::ProductConfigManager;
use snafu::{OptionExt, ResultExt, Snafu};
use stackable_operator::{
    builder::{configmap::ConfigMapBuilder, meta::ObjectMetaBuilder},
    client::Client,
    cluster_resources::{ClusterResourceApplyStrategy, ClusterResources},
    k8s_openapi::api::core::v1::ConfigMap,
    kube::{
        core::{error_boundary, DeserializeGuard},
        runtime::controller::Action,
        Resource,
    },
    kvp::ObjectLabels,
    logging::controller::ReconcilerError,
};
use strum::{EnumDiscriminants, IntoStaticStr};
use tracing::Instrument;

use crate::OPERATOR_NAME;

pub const KCL_INSTANCE_NAME: &str = "kcl-instance";
pub const KCL_INSTANCE_CONTROLLER_NAME: &str = "kcl-instance-controller";
pub const KCL_PROPERTIES_KEY: &str = "properties";
pub const KCL_MANIFESTS_KEY: &str = "manifests";

pub struct Ctx {
    pub client: Client,
    pub product_config: ProductConfigManager,
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

    #[snafu(display("object defines no namespace"))]
    ObjectHasNoNamespace,

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

    let kcl_instance_config = build_kcl_instance_config(kcl_instance, &ctx.product_config).await?;

    dbg!(&kcl_instance_config);

    cluster_resources
        .add(client, kcl_instance_config)
        .await
        .context(ApplyConfigMapSnafu)?;

    // cluster_resources
    //     .delete_orphaned_resources(client)
    //     .await
    //     .context(DeleteOrphanedResourcesSnafu)?;

    Ok(Action::await_change())
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
    _obj: Arc<DeserializeGuard<KclInstance>>,
    error: &Error,
    _ctx: Arc<Ctx>,
) -> Action {
    match error {
        // root object is invalid, will be requeued when modified anyway
        Error::InvalidKclInstance { .. } => Action::await_change(),
        _ => Action::requeue(Duration::from_secs(10)),
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
