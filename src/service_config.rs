use std::{fs::File, io::Write, path::Path};

use crate::AppState;
use actix_web::{error, web, HttpResponse, Result};
use log::{debug, error, info, warn};
use s3::{error::GetObjectErrorKind, SdkError};
use serde::Deserialize;
use tokio::sync::mpsc;
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
    let file = File::create(&path)?;

    let mut zip = zip::ZipWriter::new(file);
    let options = FileOptions::default()
        .compression_method(zip::CompressionMethod::Bzip2)
        .unix_permissions(0o755);

    let (tx, mut rx) = mpsc::channel(32);

    for key in payload.keys.iter() {
        let s3 = state.s3.clone();
        let bucket_name = String::from(state.bucket);
        let cloned_key = key.clone();
        let cloned_tx = tx.clone();

        tokio::spawn(async move {
            let object = s3
                .get_object()
                .bucket(bucket_name)
                .key(&cloned_key)
                .send()
                .await;
            warn!("send key -> {}", cloned_key);
            cloned_tx.send((cloned_key, object)).await;
        });
    }

    drop(tx);
    while let Some((key, object_output)) = rx.recv().await {
        warn!("recieved key -> {}", key);
        zip.start_file(&key, options)
            .map_err(error::ErrorInternalServerError)?;
        let mut object = object_output.map_err(|err| {
            error!("s3 get_object with key [{}]: {}", key, err);
            if let SdkError::ServiceError { err, raw: _ } = err {
                if let GetObjectErrorKind::NoSuchKey(_) = err.kind {
                    return error::ErrorBadRequest(format!("no such key [{}]", key));
                };
                return error::ErrorInternalServerError(err);
            }
            error::ErrorInternalServerError(err)
        })?;
        while let Some(bytes) = object
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
