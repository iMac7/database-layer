use serde::Serialize;

use super::abstract_entity_revision::AbstractEntityRevision;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleRevision {
    title: String,
    content: String,
    meta_title: String,
    meta_description: String,
}

impl From<&AbstractEntityRevision> for ArticleRevision {
    fn from(abstract_entity_revision: &AbstractEntityRevision) -> Self {
        let title = abstract_entity_revision.fields.get_or("title", "");
        let content = abstract_entity_revision.fields.get_or("content", "");
        let meta_title = abstract_entity_revision.fields.get_or("meta_title", "");
        let meta_description = abstract_entity_revision
            .fields
            .get_or("meta_description", "");

        ArticleRevision {
            title,
            content,
            meta_title,
            meta_description,
        }
    }
}
