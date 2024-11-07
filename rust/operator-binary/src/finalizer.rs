use flux_kcl_operator_crd::KclInstance;
use kube::{
    api::{Patch, PatchParams},
    Api, Client, Error,
};
use serde_json::{json, Value};

/// Adds a finalizer to a KclInstance resource in Kubernetes.
///
/// # Arguments
///
/// * `client` - Kubernetes client to use for the API request
/// * `name` - Name of the KclInstance resource
/// * `namespace` - Namespace containing the KclInstance
///
/// # Returns
///
/// Returns Result containing the patched KclInstance on success, or Error on failure
pub(crate) async fn add(
    client: kube::Client,
    name: &str,
    namespace: &str,
) -> Result<KclInstance, Error> {
    patch_finalizer(
        client,
        name,
        namespace,
        vec!["kcl.evrone.com/finalizer"].as_slice(),
    )
    .await
}

/// Removes a finalizer from a KclInstance resource in Kubernetes.
///
/// # Arguments
///
/// * `client` - Kubernetes client to use for the API request
/// * `name` - Name of the KclInstance resource
/// * `namespace` - Namespace containing the KclInstance
///
/// # Returns
///
/// Returns Result containing the patched KclInstance on success, or Error on failure
pub(crate) async fn delete(
    client: kube::Client,
    name: &str,
    namespace: &str,
) -> Result<KclInstance, Error> {
    patch_finalizer(client, name, namespace, vec![].as_slice()).await
}

/// Patches the list of finalizers for a KclInstance resource in Kubernetes.
///
/// # Arguments
///
/// * `client` - Kubernetes client to use for the API request
/// * `name` - Name of the KclInstance resource to patch
/// * `namespace` - Namespace containing the KclInstance
/// * `finalizers` - List of finalizer strings to set on the resource
///
/// # Returns
///
/// Returns Result containing the patched KclInstance on success, or Error on failure
async fn patch_finalizer(
    client: kube::Client,
    name: &str,
    namespace: &str,
    finalizers: &[&str],
) -> Result<KclInstance, Error> {
    let api: Api<KclInstance> = Api::namespaced(client, namespace);
    // plural.group/finalizer
    let finalizer: Value = json!({
      "metadata": {
          "finalizers": finalizers
      }
    });

    let patch: Patch<&Value> = Patch::Merge(&finalizer);
    api.patch(name, &PatchParams::default(), &patch).await
}
