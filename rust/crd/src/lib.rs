use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use k8s_openapi::{api::core::v1::ObjectReference, apimachinery::pkg::apis::meta::v1::Condition};
use kube::{
    api::{DynamicObject, GroupVersionKind},
    core::gvk::ParseGroupVersionError,
    CustomResource, ResourceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use snafu::{OptionExt, ResultExt, Snafu};

pub const APP_NAME: &str = "kcl-instance";

#[derive(Snafu, Debug)]
pub enum Error {
    #[snafu(display("object has no namespace associated"))]
    NoNamespace,

    #[snafu(display("Failed to get object key: {}", key))]
    MissingObjectKey { key: String },

    #[snafu(display("Failed to parse GVK: {}", source))]
    FailedToParseGvk { source: ParseGroupVersionError },
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

    #[serde(default)]
    pub config: KclInstanceConfig,

    pub suspend: Option<bool>,
    pub interval: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize, Default)]
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

impl TryFrom<DynamicObject> for Gvk {
    type Error = Error;

    fn try_from(value: DynamicObject) -> Result<Self, Self::Error> {
        let type_meta = value.types.clone().context(MissingObjectKeySnafu {
            key: "metadata/typeMeta",
        })?;
        let g_gvk = GroupVersionKind::try_from(&type_meta).context(FailedToParseGvkSnafu)?;

        Ok(Self {
            name: value.name_any(),
            group: g_gvk.group,
            version: g_gvk.version,
            kind: g_gvk.kind,
            namespace: value.namespace(),
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KclInstanceStatus {
    #[serde(default)]
    pub inventory: HashSet<Gvk>,
    pub observed_generation: i64,

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
