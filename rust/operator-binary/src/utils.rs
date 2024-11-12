use std::collections::BTreeMap;

use kube::{
    api::{ApiResource, DynamicObject},
    discovery::{ApiCapabilities, Scope},
    Api, Client,
};

pub fn dynamic_api(
    ar: ApiResource,
    caps: ApiCapabilities,
    client: Client,
    ns: Option<&str>,
    all: bool,
) -> Api<DynamicObject> {
    if caps.scope == Scope::Cluster || all {
        Api::all_with(client, &ar)
    } else if let Some(namespace) = ns {
        Api::namespaced_with(client, namespace, &ar)
    } else {
        Api::default_namespaced_with(client, &ar)
    }
}

pub fn multidoc_deserialize(data: &str) -> anyhow::Result<Vec<DynamicObject>> {
    use serde::Deserialize;
    let mut docs = vec![];
    for de in serde_yaml::Deserializer::from_str(data) {
        docs.push(serde_yaml::from_value(serde_yaml::Value::deserialize(de)?)?);
    }
    Ok(docs)
}

pub fn patch_labels(labels: Option<BTreeMap<String, String>>) -> Option<BTreeMap<String, String>> {
    if let Some(labels) = labels {
        let patch = BTreeMap::from([(
            "app.kubernetes.io/managed-by".to_string(),
            "kcl-instance-controller".to_string(),
        )]);
        Some(labels.into_iter().chain(patch).collect())
    } else {
        patch_labels(Some(BTreeMap::new()))
    }
}
