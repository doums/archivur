use std::io::{Cursor, Read, Write};

use crate::AppState;
use actix_web::{error, web, HttpResponse, Result};
use log::{debug, error, info};
use s3::{
    error::GetObjectErrorKind,
    model::{CompletedMultipartUpload, CompletedPart},
    ByteStream, SdkError,
};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use zip::write::FileOptions;

use std::time::Instant;

const PART_SIZE: usize = 6_000_000; // min 5 MB, max 5 GB -> see https://docs.aws.amazon.com/AmazonS3/latest/userguide/qfacts.html

#[derive(Deserialize)]
struct Payload {
    keys: Vec<String>,
}

async fn handler(
    state: web::Data<AppState<'_>>,
    payload: web::Json<Payload>,
) -> Result<HttpResponse> {
    let iteration_start: Instant;

    iteration_start = Instant::now();
    let cursor: Cursor<Vec<u8>> = Cursor::new(vec![]);
    let mut zip = zip::ZipWriter::new(cursor);
    let options = FileOptions::default()
        /* TODO
        set archive compression ? `.compression_method(zip::CompressionMethod::Bzip2)`
        set the large_file opion if zip file will be more than 4 GB.
        see https://docs.rs/zip/0.5.13/zip/write/struct.FileOptions.html#method.large_file */
        .unix_permissions(0o755);

    let (tx, mut rx) = mpsc::channel(32);

    for key in payload.keys.iter() {
        let s3 = state.s3.clone();
        let bucket_name = String::from(state.files_bucket);
        let cloned_key = key.clone();
        let cloned_tx = tx.clone();

        tokio::spawn(async move {
            let object = s3
                .get_object()
                .bucket(bucket_name)
                .key(&cloned_key)
                .send()
                .await;
            debug!("send key -> {}", cloned_key);
            cloned_tx.send((cloned_key, object)).await;
        });
    }

    drop(tx);
    while let Some((key, object_output)) = rx.recv().await {
        debug!("recieved key -> {}", key);
        zip.start_file(&key, options)
            .map_err(error::ErrorInternalServerError)?;
        let mut object = object_output.map_err(|err| {
            error!("s3 get_object with key [{}]: {}", key, err);
            if let SdkError::ServiceError { err, raw: _ } = err {
                if let GetObjectErrorKind::NoSuchKey(_) = err.kind {
                    return error::ErrorNotFound(format!("no such key [{}]", key));
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

    let mut zip = zip.finish().map_err(error::ErrorInternalServerError)?;
    zip.set_position(0);
    debug!("zip archive len [{}]", zip.get_ref().len());
    info!("zip took {}", iteration_start.elapsed().as_millis());
    /* TODO:
    check if the zip is < ~100 MB, if yes, don't use multipart_upload
    check if the zip is > 5 TB (max object size)
    check if zip_len / PART_SIZE > 10_000 (max number of parts), if yes use a bigger part size

    source: https://docs.aws.amazon.com/AmazonS3/latest/userguide/qfacts.html */

    let mut buffer: Vec<u8> = vec![0; PART_SIZE];
    let multipart_upload = state
        .s3
        .create_multipart_upload()
        .bucket(state.archives_bucket)
        .key("result.zip")
        .send()
        .await
        .map_err(error::ErrorInternalServerError)?;
    let upload_id = multipart_upload.upload_id.ok_or_else(|| {
        error!("request [CreateMultipartUpload]: no upload ID in response");
        error::ErrorInternalServerError("s3 error")
    })?;

    let mut part_number = 0;
    let (tx, mut rx) = mpsc::channel(32);
    loop {
        part_number += 1;
        let n = zip.read(&mut buffer[..PART_SIZE])?;
        if n == 0 {
            break;
        }
        debug!("readed {} bytes", n);
        let s3 = state.s3.clone();
        let archives_bucket = state.archives_bucket.to_string();
        let bytes = Vec::from(&buffer[..n]);
        let cloned_upload_id = upload_id.clone();
        let cloned_tx = tx.clone();

        tokio::spawn(async move {
            debug!(
                "start upload part [{}], size [{}] bytes",
                part_number,
                bytes.len()
            );
            match s3
                .upload_part()
                .bucket(archives_bucket)
                .key("result.zip")
                .upload_id(cloned_upload_id)
                .part_number(part_number)
                .body(ByteStream::from(bytes))
                .send()
                .await
            {
                Ok(output) => {
                    debug!(
                        "completed upload part [{}], etag {:#?}",
                        part_number, &output.e_tag
                    );
                    cloned_tx
                        .send(
                            CompletedPart::builder()
                                .set_e_tag(output.e_tag)
                                .part_number(part_number)
                                .build(),
                        )
                        .await;
                }
                Err(err) => {
                    // TODO: implement a retry logic
                    error!("request [UploadPart]: {}", err);
                }
            }
        });
    }

    drop(tx);
    let mut completed_parts: Vec<CompletedPart> = vec![];
    while let Some(part) = rx.recv().await {
        completed_parts.push(part);
    }
    completed_parts.sort_by(|a, b| a.part_number.cmp(&b.part_number));
    debug!("comleted parts {:#?}", completed_parts);

    state
        .s3
        .complete_multipart_upload()
        .bucket(state.archives_bucket)
        .key("result.zip")
        .upload_id(upload_id)
        .multipart_upload(
            CompletedMultipartUpload::builder()
                .set_parts(Some(completed_parts))
                .build(),
        )
        .send()
        .await
        .map_err(|err| {
            error!("s3 request failed [CompleteMultipartUpload]: {}", err);
            error::ErrorInternalServerError("s3 error")
        })?;

    info!("elapsed {}", iteration_start.elapsed().as_millis());
    Ok(HttpResponse::Ok().body("result.zip"))
}

pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/archive").route(web::get().to(handler)));
}
