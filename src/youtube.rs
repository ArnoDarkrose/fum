use std::sync::Mutex;
use std::time;

use actix_web::{get, web, App, HttpServer, Responder};
use bytes::Bytes;
use expanduser::expanduser;
use reqwest::Client;

use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::runtime::Runtime;
use tokio::sync::oneshot::{self, Sender};
use tokio::task::JoinHandle;
use tokio::task::JoinSet;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const CLIENT_ID: &str = "201989884872-lh4t6bs3a35ed9gug7v1njgn752igpbr.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "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";
const REDIRECT_URI: &str = "http://localhost:5000/callback";
const SCOPE: &str = "https://www.googleapis.com/auth/youtube.force-ssl";
const CODE_EXCHANGE_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(serde::Deserialize)]
struct CallbackResponse {
    error: Option<String>,
    code: Option<String>,
}

struct AppData {
    sender: Mutex<Option<Sender<JoinHandle<()>>>>,
}

#[get("/callback")]
async fn callback(
    mut query: web::Query<CallbackResponse>,
    app_data: web::Data<AppData>,
) -> impl Responder {
    if let Some(error) = &query.error {
        panic!("got resonse {error}");
    }

    let code = query.code.take().unwrap();

    let rest_tasks = tokio::spawn(get_tokens(code));

    let mut sender = app_data.sender.lock().expect("mutex poisoned");
    sender.take().unwrap().send(rest_tasks).unwrap();

    return "Successfully handled google callback. You can close this page now";
}

#[derive(serde::Serialize)]
struct ExchangeTokensQuery {
    client_id: String,
    client_secret: String,
    code: String,
    grant_type: String,
    redirect_uri: String,
}

#[derive(serde::Deserialize)]
struct ExchangeTokensResponse {
    access_token: String,
    expires_in: usize,
    #[serde(rename = "id_token")]
    _id_token: Option<String>,
    refresh_token: String,
    #[serde(rename = "scope")]
    _scope: String,
    #[serde(rename = "token_type")]
    _token_type: String,
}

async fn get_tokens(code: String) {
    let client = Client::new();

    let response = client
        .post(CODE_EXCHANGE_URL)
        .json(&ExchangeTokensQuery {
            client_id: CLIENT_ID.to_string(),
            client_secret: CLIENT_SECRET.to_string(),
            code,
            grant_type: "authorization_code".to_string(),
            redirect_uri: REDIRECT_URI.to_string(),
        })
        .send()
        .await
        .unwrap();

    let response: ExchangeTokensResponse = response.json().await.unwrap();

    let expiration_date = (time::SystemTime::now()
        + time::Duration::from_secs(response.expires_in as u64))
    .duration_since(time::UNIX_EPOCH)
    .expect("current time is set earlier than UNIX_EPOCH")
    .as_secs();

    let base_path = expanduser("~/.config/fum").expect("fum config dir not found");

    fn save_data<T: std::fmt::Display + Send + Sync + 'static>(
        set: &mut JoinSet<()>,
        data: T,
        filename: &'static str,
        base_path: std::path::PathBuf,
    ) {
        let mut path = base_path.clone();
        path.push(filename);

        set.spawn(async move {
            let mut fd = File::create(&path).await.expect("fum config dir not found");

            fd.write_all(format!("{data}").as_bytes())
                .await
                .expect("failed to write to file");
        });
    }

    let mut set = JoinSet::new();

    save_data(
        &mut set,
        response.access_token,
        "access_token",
        base_path.clone(),
    );

    save_data(
        &mut set,
        response.refresh_token,
        "refresh_token",
        base_path.clone(),
    );

    save_data(
        &mut set,
        expiration_date,
        "access_token_expiration_date",
        base_path,
    );

    set.join_all().await;
}

async fn start() {
    let (sender, receiver) = oneshot::channel();
    let server = tokio::spawn(start_server(Mutex::new(Some(sender))));

    let auth_url = format!(
        "{AUTH_URL}?client_id={CLIENT_ID}&redirect_uri={REDIRECT_URI}&scope={SCOPE}&response_type=code&access_type=offline"
    );

    webbrowser::open(&auth_url).expect("failed to open google authenticate url");

    let rest_tasks_handle = receiver.await.unwrap();
    server.abort();
    rest_tasks_handle.await.unwrap();
}

async fn start_server(sender: Mutex<Option<Sender<JoinHandle<()>>>>) {
    let app_data = web::Data::new(AppData { sender });
    HttpServer::new(move || {
        App::new()
            // .service(home)
            .service(callback)
            .app_data(app_data.clone())
    })
    .bind(("127.0.0.1", 5000))
    .expect("failed to bind http server to local port")
    .run()
    .await
    .expect("failed to launch server");
}

pub fn authorize() {
    let rt = Runtime::new().expect("failed to start tokio runtime");

    rt.block_on(start());
}
