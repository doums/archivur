mod service_config;
use actix_web::http::Uri;
use actix_web::middleware::Logger;
use actix_web::{rt::System, web, App, HttpServer};
use aws_auth::Credentials;
use aws_auth::provider::{CredentialsError, ProvideCredentials};
use log::info;
use s3::{Config, Region};
use smithy_http::endpoint::Endpoint;
use std::env::set_var;
use std::error::Error;
use std::sync::mpsc;
use std::thread;

use crate::service_config::config;

const HOST: &str = "localhost";
const PORT: u32 = 4000;
const S3_HOST: &str = "http://localhost:9000";
const AWS_ACCESS_KEY_ID: &str = "minio";
const AWS_SECRET_ACCESS_KEY: &str = "minio123";
const S3_BUCKET_FILES: &str = "files";
const S3_BUCKET_ARCHIVES: &str = "archives";

#[derive(Debug)]
struct AppState<'a> {
    s3: s3::Client,
    files_bucket: &'a str,
    archives_bucket: &'a str,
}

impl<'a> AppState<'a> {
    fn new(client: s3::Client, files_bucket: &'a str, archives_bucket: &'a str) -> Self {
        AppState {
            s3: client,
            files_bucket,
            archives_bucket,
        }
    }
}

struct CredentialsProvider;

impl ProvideCredentials for CredentialsProvider {
    fn provide_credentials(&self) -> Result<Credentials, CredentialsError> {
        Ok(Credentials::from_keys(
            AWS_ACCESS_KEY_ID,
            AWS_SECRET_ACCESS_KEY,
            None,
        ))
    }
}

#[actix_web::main]
async fn main() -> Result<(), Box<dyn Error>> {
    set_var("RUST_LOG", "error,archivur=debug");
    env_logger::init();

    let region = Region::new("eu-west-3");
    let s3_config = Config::builder()
        .region(region)
        .endpoint_resolver(Endpoint::immutable(Uri::from_static(S3_HOST)))
        .credentials_provider(CredentialsProvider)
        .build();
    let client = s3::Client::from_conf(s3_config);

    let app_state = web::Data::new(AppState::new(client, S3_BUCKET_FILES, S3_BUCKET_ARCHIVES));
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let sys = System::new();
        sys.block_on(async {
            let srv = HttpServer::new(move || {
                App::new()
                    .wrap(Logger::new("%r -> [%s] took %Tms"))
                    .app_data(app_state.clone())
                    .configure(config)
            })
            .bind(format!("{}:{}", HOST, PORT))
            .unwrap()
            .shutdown_timeout(5)
            .run();

            let _ = tx.send(srv);
        });
        info!("🦀 archivur running on port {} 🦀", PORT);
        sys.run()
    });

    let srv = rx.recv().unwrap();
    srv.await?;
    info!("archivur gracefully shutdown");
    Ok(())
}
