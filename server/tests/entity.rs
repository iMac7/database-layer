#[cfg(test)]
mod tests {
    use actix_web::{test, App};
    use std::str::from_utf8;

    use server::{configure_app, create_database_pool};

    #[actix_rt::test]
    async fn unrevised_entities_query() {
        let pool = create_database_pool().await.unwrap();
        let app = configure_app(App::new(), pool);
        let app = test::init_service(app).await;
        let req = test::TestRequest::post()
            .uri("/")
            .set_json(&serde_json::json!({
                "type": "UnrevisedEntitiesQuery",
                "payload": {}
            }))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert!(resp.status().is_success());

        let result = json::parse(from_utf8(&test::read_body(resp).await).unwrap()).unwrap();

        assert_eq!(
            result,
            json::object! {
                "unrevisedEntityIds": [
                    26892,
                    33582,
                    34741,
                    34907,
                    35247,
                    35556
                 ]
            }
        );
    }
}