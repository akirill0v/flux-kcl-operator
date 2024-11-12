use std::{
    borrow::BorrowMut,
    collections::BTreeMap,
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
};

use flux_kcl_operator_crd::{KclInstance, KclInstanceStatus};
use fluxcd_rs::{Downloader, FluxSourceArtefact, GitRepository, OCIRepository};

use kcl_client::ModClient;
use kube::{
    api::{
        DeleteParams, DynamicObject, GroupVersionKind, ListParams, Patch, PatchParams,
        Preconditions,
    },
    core::gvk::ParseGroupVersionError,
    Api, Client, Discovery, ResourceExt,
};
use snafu::{OptionExt, ResultExt, Snafu};
use strum::{EnumDiscriminants, IntoStaticStr};
use tracing::{error, info, warn};

use crate::utils::{self, patch_labels};

static OPERATOR_MANAGER: &str = "kcl-instance-controller";

#[derive(Snafu, Debug, EnumDiscriminants)]
#[strum_discriminants(derive(IntoStaticStr))]
#[allow(clippy::enum_variant_names)]
pub enum Error {
    #[snafu(display("Failed to download: {}", source))]
    DownloadError {
        source: fluxcd_rs::downloader::error::DownloaderError,
    },

    #[snafu(display("Failed to get artefact: {}", name))]
    ArtefactMissing { name: String },

    #[snafu(display("object defines no name"))]
    ObjectHasNoName,

    #[snafu(display("object defines no config"))]
    ObjectHasNoConfig,

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

    #[snafu(display("failed to find kubernetes object"))]
    ObjectHasNotFound { source: kube::Error },

    #[snafu(display("Failed to make kcl client actions: {}", source))]
    KclClientActions { source: kcl_client::Error },

    #[snafu(display("Failed to compile package: {}", source))]
    CompilePackage { source: anyhow::Error },

    #[snafu(display("Failed to apply KCL module: {}", source))]
    ApplyYamlManifests { source: kube::Error },

    #[snafu(display("Failed to apply KCL status: {}", source))]
    ApplyYamlStatus { source: kube::Error },

    #[snafu(display("Failed deserialize yaml manifests: {}", source))]
    WrongYamlManifests { source: serde_yaml::Error },

    #[snafu(display("Failed to get gvk from obj: {:?}", obj))]
    NoManagedTypeInDynamicObject { obj: DynamicObject },

    #[snafu(display("Failed to get gvk from manifests: {}", source))]
    FailedToGetGvk { source: ParseGroupVersionError },

    #[snafu(display("Failed to parse group version for: {}", name))]
    ParseGroupVersion { name: String },

    #[snafu(display("Failed to deserialize manifests: {}", source))]
    UnableToDeserialize { source: serde_json::Error },

    #[snafu(display("Failed to patch KCL module: {}", source))]
    FailedToPatch { source: kube::Error },

    #[snafu(display("KCL instance {} is missing status", name))]
    KclInstanceMissingStatus { name: String },

    #[snafu(display("Failed to delete resource: {}", source))]
    FailedToDelete { source: kube::Error },
}

type Result<T, E = Error> = std::result::Result<T, E>;

/// An Engine is a component that executes KCL configurations against a Kubernetes cluster.
///
/// The Engine handles:
/// - Downloading KCL source files from Git/OCI repositories
/// - Rendering KCL files with provided arguments
/// - Interfacing with the Kubernetes API
pub struct Engine {
    client: Client,
}

impl Engine {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub(crate) async fn cleanup(
        &self,
        instance: Arc<KclInstance>,
        discovery: &Discovery,
    ) -> Result<()> {
        for item in instance
            .status
            .as_ref()
            .context(KclInstanceMissingStatusSnafu {
                name: instance.name_any(),
            })?
            .inventory
            .iter()
        {
            let gvk = GroupVersionKind {
                group: item.group.clone(),
                version: item.version.clone(),
                kind: item.kind.clone(),
            };

            info!(
                "Prepare to deleting resource: {} with name: {}",
                gvk.kind, item.name
            );

            // Resolve the API resource and capabilities for this GVK
            if let Some((ar, caps)) = discovery.resolve_gvk(&gvk) {
                let delete_params = DeleteParams::default();

                // Create a dynamic API client for this resource type
                let api = crate::utils::dynamic_api(
                    ar,
                    caps,
                    self.client.clone(),
                    item.namespace.as_deref(),
                    false,
                );

                if let Ok(res) = api.get(&item.name).await {
                    if !utils::is_managed_by(OPERATOR_MANAGER, res.metadata) {
                        warn!("Skipping unmanaged resource: {}", item.name);
                        continue;
                    }
                }

                let _ = api.delete(&item.name, &delete_params).await.map_err(|e| {
                    error!("Cleanup failed: {}", e);
                    e
                });
            } else {
                warn!("Failed to resolve gvk: {:?}", gvk);
            }
        }

        Ok(())
    }

    /// Applies a Kubernetes manifest to the cluster
    ///
    /// # Arguments
    /// * `manifest` - The YAML manifest to apply
    /// * `default_namespace` - The default namespace to use if not specified in the manifest
    /// * `discovery` - Kubernetes API discovery client
    ///
    /// # Returns
    /// The applied DynamicObject or an error
    pub(crate) async fn apply(
        &self,
        obj: DynamicObject,
        default_namespace: &str,
        discovery: &Discovery,
    ) -> Result<DynamicObject> {
        let mut obj = obj;
        // Extract the name and namespace from the object
        let name = obj.name_any();
        let namespace = obj
            .metadata
            .namespace
            .as_deref()
            .unwrap_or(default_namespace);

        obj.metadata.labels = patch_labels(obj.metadata.labels.clone(), OPERATOR_MANAGER);

        // Get the GroupVersionKind (GVK) from the object's type metadata
        let gvk = obj
            .types
            .as_ref()
            .map(GroupVersionKind::try_from)
            .context(NoManagedTypeInDynamicObjectSnafu { obj: obj.clone() })?
            .context(FailedToGetGvkSnafu)?;

        // Resolve the API resource and capabilities for this GVK
        let (ar, caps) = discovery
            .resolve_gvk(&gvk)
            .context(ParseGroupVersionSnafu { name: &name })?;

        // Create patch parameters for server-side apply
        let pp = PatchParams::apply(OPERATOR_MANAGER);

        // Create a dynamic API client for this resource type
        let api = crate::utils::dynamic_api(ar, caps, self.client.clone(), Some(namespace), false);

        // Convert the object to JSON for patching
        let data: serde_json::Value =
            serde_json::to_value(&obj).context(UnableToDeserializeSnafu)?;
        info!("Apply manifest: \n{:?}", data);

        // Apply the patch to the cluster
        api.patch(&name, &pp, &Patch::Apply(&data))
            .await
            .context(FailedToPatchSnafu)
    }

    pub(crate) async fn render(
        &self,
        instance: Arc<KclInstance>,
        work_dir: &Path,
    ) -> Result<String> {
        // Creates a new ModClient instance with the specified work directory path
        let mut mod_client =
            ModClient::new(work_dir.join(&instance.spec.path)).context(KclClientActionsSnafu)?;

        // Resolves all dependencies for the KCL configuration
        let metadata = mod_client
            .resolve_all_deps(true)
            .await
            .context(KclClientActionsSnafu)?;

        // Executes the KCL compiler with resolved metadata and instance arguments
        let manifests = mod_client
            .run(
                metadata,
                instance
                    .spec
                    .instance_config
                    .clone()
                    .context(ObjectHasNoConfigSnafu)?
                    .arguments,
            )
            .await
            .context(KclClientActionsSnafu)?;
        Ok(manifests)
    }

    pub(crate) async fn download(
        &self,
        instance: Arc<KclInstance>,
        downloader: &Downloader,
    ) -> Result<PathBuf> {
        let source = &instance.spec.source;
        let source_name = source.name.as_ref().context(ObjectHasNoNameSnafu)?;
        let source_namespace = source
            .namespace
            .as_ref()
            .or(instance.metadata.namespace.as_ref())
            .context(ObjectHasNoNamespaceSnafu)?;

        let artefact = self.get_artefact(&instance).await?;
        downloader
            .download(&artefact.url(), source_name, source_namespace)
            .await
            .context(DownloadSnafu)
    }

    async fn get_artefact(&self, instance: &KclInstance) -> Result<FluxSourceArtefact> {
        let source = &instance.spec.source;
        let source_name = source.name.as_ref().context(ObjectHasNoNameSnafu)?;
        let source_namespace = source
            .namespace
            .as_ref()
            .or(instance.metadata.namespace.as_ref())
            .context(ObjectHasNoNamespaceSnafu)?;

        match source.kind.as_deref() {
            Some("GitRepository") => Ok(FluxSourceArtefact::Git(
                Api::<GitRepository>::namespaced(self.client.clone(), source_namespace)
                    .get(source_name)
                    .await
                    .context(ObjectHasNotFoundSnafu)?
                    .status
                    .context(ObjectHasNoStatusSnafu)?
                    .artifact
                    .context(ObjectHasNoArtefactSnafu)?,
            )),
            Some("OciRepository") => Ok(FluxSourceArtefact::Oci(
                Api::<OCIRepository>::namespaced(self.client.clone(), source_namespace)
                    .get(source_name)
                    .await
                    .context(ObjectHasNotFoundSnafu)?
                    .status
                    .context(ObjectHasNoStatusSnafu)?
                    .artifact
                    .context(ObjectHasNoArtefactSnafu)?,
            )),
            _ => Err(Error::ObjectHasNoKind),
        }
    }

    pub(crate) async fn update_status(
        &self,
        instance: Arc<KclInstance>,
        status: KclInstanceStatus,
    ) -> Result<KclInstance> {
        let api = Api::<KclInstance>::namespaced(
            self.client.clone(),
            &instance.namespace().context(ObjectHasNoNamespaceSnafu)?,
        );

        let current = api
            .get(&instance.name_any())
            .await
            .context(ObjectHasNotFoundSnafu)?;
        let mut instance_imt = current.clone();
        instance_imt.status = Some(status);

        // Create patch parameters for server-side apply
        let pp = PatchParams::apply(OPERATOR_MANAGER).validation_strict();

        api.patch_status(&instance.name_any(), &pp, &Patch::Merge(&instance_imt))
            .await
            .context(ApplyYamlStatusSnafu)
    }
}