pub mod git_repository;
pub mod oci_repository;

pub use git_repository::*;
pub use oci_repository::*;

#[derive(Debug, Clone)]
pub enum FluxSourceArtefact {
    Git(GitRepositoryStatusArtifact),
    Oci(OCIRepositoryStatusArtifact),
}

impl FluxSourceArtefact {
    pub fn url(&self) -> String {
        match self {
            FluxSourceArtefact::Git(artefact) => artefact.url.clone(),
            FluxSourceArtefact::Oci(artefact) => artefact.url.clone(),
        }
    }
}
