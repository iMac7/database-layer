use serde::Serialize;

use super::abstract_entity_revision::AbstractEntityRevision;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoursePageRevision {
    title: String,
    content: String,
}

impl From<&AbstractEntityRevision> for CoursePageRevision {
    fn from(abstract_entity_revision: &AbstractEntityRevision) -> Self {
        let title = abstract_entity_revision.fields.get_or("title", "");
        let content = abstract_entity_revision.fields.get_or("content", "");

        CoursePageRevision { title, content }
    }
}
