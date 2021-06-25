use actix_web::{web, HttpResponse};
use serde::Deserialize;

use crate::AppState;

#[derive(Deserialize)]
struct Payload {
    files: Vec<String>,
}

async fn handler(data: web::Data<AppState>, payload: web::Json<Payload>) -> HttpResponse {
    println!("data -> {:#?}", data.0);
    let mut i = data.0.lock().unwrap();
    *i += 1;
    println!("data -> {:#?}", data.0);
    for file in &payload.files {
        println!("the value is: {}", file);
    }
    HttpResponse::Ok().body("OK")
}

pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/archive").route(web::get().to(handler)));
}
