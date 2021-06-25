mod service_config;
use actix_web::http::Uri;
use actix_web::middleware::Logger;
use actix_web::{rt::System, web, App, HttpServer};
use aws_auth::Credentials;
use aws_auth::CredentialsError;
use aws_auth::ProvideCredentials;
use log::info;
use s3::{Config, Region};
use smithy_http::endpoint::Endpoint;
use std::env::set_var;
use std::error::Error;
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;

use crate::service_config::config;

const HOST: &str = "localhost";
const PORT: u32 = 4000;
const S3_HOST: &str = "http://localhost:9000";
const AWS_ACCESS_KEY_ID: &str = "minio";
const AWS_SECRET_ACCESS_KEY: &str = "minio123";

#[derive(Debug)]
struct AppState(Mutex<i32>);

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    set_var("RUST_LOG", "debug,actix_web=debug");
    env_logger::init();
    let app_state = web::Data::new(AppState(Mutex::new(0)));
    let region = Region::new("eu-west-3");
    let s3_config = Config::builder()
        .region(region)
        .endpoint_resolver(Endpoint::immutable(Uri::from_static(S3_HOST)))
        .credentials_provider(CredentialsProvider)
        .build();
    let client = s3::Client::from_conf(s3_config);
    let resp = client.list_buckets().send().await?;

    for bucket in resp.buckets.unwrap_or_default() {
        println!("bucket: {:?}", bucket.name)
    }

    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let sys = System::new("archivur");

        let srv = HttpServer::new(move || {
            App::new()
                .wrap(Logger::new("%r -> [%s] took %Tms"))
                .app_data(app_state.clone())
                .configure(config)
        })
        .bind(format!("{}:{}", HOST, PORT))?
        .shutdown_timeout(5)
        .run();

        let _ = tx.send(srv);
        info!("🦀 archivur running on port {} 🦀", PORT);
        sys.run()
    });

    let srv = rx.recv().unwrap();
    srv.await?;
    info!("archivur gracefully shutdown");
    Ok(())
}
