use std::collections::HashMap;

use k8s_openapi::api::core::v1::ObjectReference;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use snafu::{OptionExt, ResultExt, Snafu};
use stackable_operator::{
    commons::cluster_operation::ClusterOperation,
    kube::{runtime::reflector::ObjectRef, CustomResource, ResourceExt},
    schemars::{self, JsonSchema},
    status::condition::ClusterCondition,
};

pub const APP_NAME: &str = "kcl-instance";

#[derive(Snafu, Debug)]
pub enum Error {
    #[snafu(display("object has no namespace associated"))]
    NoNamespace,
}

#[derive(Clone, CustomResource, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[kube(
    group = "kcl.evrone.com",
    version = "v1alpha1",
    kind = "KclInstance",
    shortname = "ki",
    status = "KclInstanceStatus",
    namespaced,
    crates(
        kube_core = "stackable_operator::kube::core",
        k8s_openapi = "stackable_operator::k8s_openapi",
        schemars = "stackable_operator::schemars"
    )
)]
#[serde(rename_all = "camelCase")]
pub struct KclInstanceSpec {
    #[serde(rename = "sourceRef")]
    pub source: ObjectReference,
    pub path: String,
    pub instance_config: Option<KclInstanceConfig>,

    // no doc - docs in ClusterOperation struct.
    #[serde(default)]
    pub cluster_operation: ClusterOperation,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KclInstanceConfig {
    pub vendor: bool,
    pub sort_keys: bool,
    pub show_hidden: bool,
    pub arguments: HashMap<String, String>,
    pub arguments_from: Vec<ArgumentsReference>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArgumentsReference {
    /// Name of the values referent. Should reside in the same namespace as the referring resource.
    pub name: String,

    /// Kind of the values referent, valid values are (‘Secret’, ‘ConfigMap’).
    pub kind: String,

    /// ArgumentsKey is the data key where the arguments.yaml or a specific value can be found at.
    /// Defaults to ‘arguments.yaml’.
    pub arguments_key: Option<String>,

    /// TargetPath is the YAML dot notation path the value should be merged at.
    /// When set, the ArgumentsKey is expected to be a single flat value.
    /// Defaults to ‘None’, which results in the values getting merged at the root.
    pub target_path: Option<String>,

    /// Optional marks this ArgumentsReference as optional.
    /// When set, a not found error for the values reference is ignored, but any ArgumentsKey,
    /// TargetPath or transient error will still result in a reconciliation failure.
    /// Defaults to false.
    #[serde(default)]
    pub optional: bool,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KclInstanceStatus {
    #[serde(default)]
    pub inventory: Vec<String>,

    pub last_applied_revision: Option<String>,
    pub last_attempted_revision: Option<String>,

    #[serde(default)]
    pub conditions: Vec<ClusterCondition>,
}
