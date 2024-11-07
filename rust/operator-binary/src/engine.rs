use std::{path::PathBuf, sync::Arc};

use flux_kcl_operator_crd::KclInstance;
use fluxcd_rs::{Downloader, FluxSourceArtefact, GitRepository, OCIRepository};

use kube::{Api, Client};
use snafu::{OptionExt, ResultExt, Snafu};
use strum::{EnumDiscriminants, IntoStaticStr};

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
}

type Result<T, E = Error> = std::result::Result<T, E>;

pub struct Engine {
    client: Client,
}

impl Engine {
    pub fn new(client: Client) -> Self {
        Self { client }
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
}
