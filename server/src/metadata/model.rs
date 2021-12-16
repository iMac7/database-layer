use crate::database::Executor;
use serde::Serialize;
use serde_json::json;

use super::messages::entities_metadata_query::Payload;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityMetadata {
    #[serde(rename = "@context")]
    context: serde_json::Value,
    id: String,
    #[serde(rename = "type")]
    schema_type: Vec<String>,
    date_created: String,
    date_modified: String,
    description: Option<String>,
    headline: Option<String>,
    identifier: serde_json::Value,
    in_language: String,
    is_accessible_for_free: bool,
    is_family_friendly: bool,
    learning_resource_type: String,
    license: serde_json::Value,
    maintainer: String,
    name: String,
    publisher: String,
    version: String,
}

impl EntityMetadata {
    pub async fn query<'a, E>(
        payload: &Payload,
        executor: E,
    ) -> Result<Vec<EntityMetadata>, sqlx::Error>
    where
        E: Executor<'a>,
    {
        Ok(sqlx::query!(
            r#"
                SELECT
                    entity.id,
                    type.name AS resource_type,
                    JSON_OBJECTAGG(entity_revision_field.field, entity_revision_field.value) AS params,
                    entity.date AS date_created,
                    entity_revision.date AS date_modified,
                    entity.current_revision_id AS version,
                    license.url AS license_url,
                    instance.subdomain AS instance
                FROM entity
                JOIN uuid ON uuid.id = entity.id
                JOIN instance ON entity.instance_id = instance.id
                JOIN type on entity.type_id = type.id
                JOIN license on license.id = entity.license_id
                JOIN entity_revision ON entity.current_revision_id = entity_revision.id
                JOIN entity_revision_field on entity_revision_field.entity_revision_id = entity_revision.id
                WHERE entity.id > ?
                    AND (? is NULL OR instance.subdomain = ?)
                    AND (? is NULL OR entity_revision.date > ?)
                    AND uuid.trashed = 0
                    AND entity.type_id IN (48, 3, 7, 1, 4, 6)
                GROUP BY entity.id
                ORDER BY entity.id
                LIMIT ?
            "#,
            payload.after.unwrap_or(0),
            payload.instance,
            payload.instance,
            payload.modified_after,
            payload.modified_after,
            payload.first
        ).fetch_all(executor)
            .await?
            .into_iter()
            .map(|result| {
                let title: Option<String> = result.params.as_ref()
                    .and_then(|params| params.get("title"))
                    .and_then(|title| title.as_str())
                    .map(|title| title.to_string());
                let id = get_iri(result.id as i32);
                let learning_resource_type = get_learning_resource_type(&result.resource_type);
                let name = title.clone().unwrap_or_else(|| format!("{}: {}", learning_resource_type, id));

                EntityMetadata {
                    context: json!([
                        "https://w3id.org/kim/lrmi-profile/draft/context.jsonld",
                        { "@language": result.instance }
                    ]),
                    schema_type: vec![
                        "LearningResource".to_string(),
                        get_learning_resource_type(&result.resource_type)
                    ],
                    description: result.params.as_ref()
                        .and_then(|params| params.get("meta_description"))
                        .and_then(|title| title.as_str())
                        .map(|title| title.to_string()),
                    date_created: result.date_created.to_rfc3339(),
                    date_modified: result.date_modified.to_rfc3339(),
                    headline: title,
                    id,
                    identifier: json!({
                        "type": "PropertyValue",
                        "propertyID": "UUID",
                        "value": result.id as i32,
                    }),
                    in_language: result.instance,
                    is_accessible_for_free: true,
                    is_family_friendly: true,
                    learning_resource_type,
                    license: json!({"id": result.license_url}),
                    maintainer: "https://serlo.org/".to_string(),
                    name,
                    publisher: "https://serlo.org/".to_string(),
                    version: get_iri(result.version.unwrap())
                }
            })
            .collect()
        )
    }
}

fn get_iri(id: i32) -> String {
    format!("https://serlo.org/{}", id)
}

fn get_learning_resource_type(entity_type: &str) -> String {
    match entity_type {
        "article" | "course-page" => "Article",
        "course" => "Course",
        "text-exercise-group" | "text-exercise" => "Quiz",
        "video" => "Video",
        _ => "",
    }
    .to_string()
}