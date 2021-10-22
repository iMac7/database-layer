use serde::Serialize;

use super::abstract_entity_revision::AbstractEntityRevision;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenericRevision {
    content: String,
}

impl From<&AbstractEntityRevision> for GenericRevision {
    fn from(abstract_entity_revision: &AbstractEntityRevision) -> Self {
        let content = abstract_entity_revision.fields.get_or("content", "");

        GenericRevision { content }
    }
}
