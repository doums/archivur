use std::{fs::File, io::Write, path::Path};

use crate::AppState;
use actix_web::{error, web, HttpResponse, Result};
use log::{debug, error};
use s3::{error::GetObjectErrorKind, SdkError};
use serde::Deserialize;
use tokio_stream::StreamExt;
use zip::write::FileOptions;

#[derive(Deserialize)]
struct Payload {
    keys: Vec<String>,
}

async fn handler(
    state: web::Data<AppState<'_>>,
    payload: web::Json<Payload>,
) -> Result<HttpResponse> {
    let raw_path = "archive/test.zip";
    let path = Path::new(&raw_path);
    let file = File::create(&path).unwrap();

    let mut zip = zip::ZipWriter::new(file);
    let options = FileOptions::default()
        .compression_method(zip::CompressionMethod::Bzip2)
        .unix_permissions(0o755);

    for key in payload.keys.iter() {
        let mut obj = state
            .s3
            .get_object()
            .bucket(state.bucket)
            .key(key)
            .send()
            .await
            .map_err(|err| {
                error!("s3 get_object with key [{}]: {}", key, err);
                if let SdkError::ServiceError { err, raw: _ } = err {
                    if let GetObjectErrorKind::NoSuchKey(_) = err.kind {
                        return error::ErrorBadRequest(format!("no such key [{}]", key));
                    };
                    return error::ErrorInternalServerError(err);
                }
                error::ErrorInternalServerError(err)
            })?;
        debug!("obj -> {:#?}", obj);
        zip.start_file(key, options)
            .map_err(error::ErrorInternalServerError)?;
        while let Some(bytes) = obj
            .body
            .try_next()
            .await
            .map_err(error::ErrorInternalServerError)?
        {
            zip.write_all(&bytes)?;
        }
    }
    zip.finish().map_err(error::ErrorInternalServerError)?;
    Ok(HttpResponse::Ok().body("OK"))
}

pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/archive").route(web::get().to(handler)));
}
