use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use k8s_openapi::{api::core::v1::ObjectReference, apimachinery::pkg::apis::meta::v1::Condition};
use kube::{api::ObjectMeta, CustomResource};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use snafu::Snafu;

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
    plural = "kclinstances",
    kind = "KclInstance",
    shortname = "ki",
    status = "KclInstanceStatus",
    namespaced
)]
#[serde(rename_all = "camelCase")]
pub struct KclInstanceSpec {
    #[serde(rename = "sourceRef")]
    pub source: ObjectReference,
    pub path: String,
    pub instance_config: Option<KclInstanceConfig>,

    pub interval: Option<String>,
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

#[derive(Deserialize, Serialize, Clone, Default, Debug, Eq, PartialEq, Hash, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Gvk {
    pub name: String,
    pub group: String,
    pub version: String,
    pub kind: String,
    pub namespace: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KclInstanceStatus {
    #[serde(default)]
    pub inventory: HashSet<Gvk>,

    pub last_applied_revision: Option<String>,
    pub last_attempted_revision: Option<String>,

    /// Conditions holds the conditions for the KclInstance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Vec<Condition>>,
}

impl KclInstance {
    pub fn interval(&self) -> std::time::Duration {
        if let Some(interval) = &self.spec.interval {
            humantime::parse_duration(interval).unwrap_or(Duration::from_secs(10))
        } else {
            Duration::from_secs(10)
        }
    }
}
