use actix_web::{error, web, HttpResponse, Result};
use serde::Deserialize;
use log::debug;

use crate::AppState;

#[derive(Deserialize)]
struct Payload {
    files: Vec<String>,
}

async fn handler(data: web::Data<AppState>, payload: web::Json<Payload>) -> Result<HttpResponse> {
    let resp = data
        .s3
        .list_buckets()
        .send()
        .await
        .map_err(error::ErrorInternalServerError)?;

    for bucket in resp.buckets.unwrap_or_default() {
        debug!("bucket: {:?}", bucket.name)
    }
    Ok(HttpResponse::Ok().body("OK"))
}

pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/archive").route(web::get().to(handler)));
}
