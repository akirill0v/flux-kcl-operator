use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use flux_kcl_operator_crd::{ArgumentsReferenceKind, KclInstance};
use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use kube::{Api, Client};

use snafu::Snafu;
use strum::{EnumDiscriminants, IntoStaticStr};

#[derive(Snafu, Debug, EnumDiscriminants)]
#[strum_discriminants(derive(IntoStaticStr))]
#[allow(clippy::enum_variant_names)]
pub enum Error {
    #[snafu(display("Failed to get arguments from reference {}", name))]
    MissingArguments { name: String },
}

type Result<T, E = Error> = std::result::Result<T, E>;

#[async_trait]
pub trait InstanceExt {
    async fn get_all_args(
        &self,
        client: &Client,
        namespace: &str,
    ) -> Result<HashMap<String, String>>;
}

#[async_trait]
impl InstanceExt for KclInstance {
    async fn get_all_args(
        &self,
        client: &Client,
        namespace: &str,
    ) -> Result<HashMap<String, String>> {
        let mut args: HashMap<String, String> = self.spec.config.arguments.clone();
        for arg_ref in self.spec.config.arguments_from.iter() {
            let map = match arg_ref.kind {
                ArgumentsReferenceKind::Secret => {
                    Api::<Secret>::namespaced(client.clone(), namespace)
                        .get(&arg_ref.name)
                        .await
                        .map(|i| {
                            let mut data = BTreeMap::new();
                            data.extend(i.string_data.unwrap_or_default());

                            data.extend(
                                i.data
                                    .unwrap_or_default()
                                    .into_iter()
                                    .map(|(k, v)| (k, String::from_utf8_lossy(&v.0).to_string())),
                            );
                            data
                        })
                }
                ArgumentsReferenceKind::ConfigMap => {
                    Api::<ConfigMap>::namespaced(client.clone(), namespace)
                        .get(&arg_ref.name)
                        .await
                        .map(|i| i.data.unwrap_or_default())
                }
            };

            if let Ok(map) = map {
                args.extend(map);
            } else if !arg_ref.optional {
                return MissingArgumentsSnafu {
                    name: &arg_ref.name,
                }
                .fail();
            }
        }
        return Ok(args);
    }
}
