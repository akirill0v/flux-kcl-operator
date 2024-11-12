use std::sync::Arc;

use flux_kcl_operator_crd::KclInstance;
use kube::{
    runtime::{
        events::{Event, EventType, Recorder, Reporter},
        reflector::ObjectRef,
    },
    Client,
};

use snafu::{OptionExt, ResultExt, Snafu};
use strum::{EnumDiscriminants, IntoStaticStr};

#[derive(Snafu, Debug, EnumDiscriminants)]
#[strum_discriminants(derive(IntoStaticStr))]
#[allow(clippy::enum_variant_names)]
pub enum Error {
    #[snafu(display("Failed to publish event: {}", source))]
    PublishEvent { source: kube::Error },
}

pub async fn publish_event(
    instance: Arc<KclInstance>,
    client: Client,
    action: String,
    reason: String,
    note: Option<String>,
) -> Result<(), Error> {
    let reporter: Reporter = crate::engine::OPERATOR_MANAGER.into();

    let object_ref = ObjectRef::from_obj(instance.as_ref());

    let recorder = Recorder::new(client.to_owned(), reporter, object_ref.into());
    recorder
        .publish(Event {
            action,
            reason,
            note,
            type_: EventType::Warning,
            secondary: None,
        })
        .await
        .context(PublishEventSnafu)
}
