use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{self, Duration, SystemTime};

use actix_web::{get, web, App, HttpServer, Responder};
use expanduser::expanduser;
use reqwest::Client;

use tokio::fs::File;
use tokio::io::AsyncReadExt;
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
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const RATE_URL: &str = "https://www.googleapis.com/youtube/v3/videos/rate";

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

    "Successfully handled google callback. You can close this page now"
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
    refresh_token: Option<String>,
    #[serde(rename = "scope")]
    _scope: String,
    #[serde(rename = "token_type")]
    _token_type: String,
}

async fn get_tokens(code: String) {
    let client = Client::new();

    let response = client
        .post(TOKEN_URL)
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
        response
            .refresh_token
            .expect("this request always send refresh token"),
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

pub fn extract_video_id(url: &str) -> Option<String> {
    let regex = regex::Regex::new(r"(?:v=|\/)([0-9A-Za-z_-]{11}).*").expect("never fails");

    regex
        .captures(url)
        .and_then(|capture| capture.get(1).map(|m| m.as_str().to_string()))
}

#[derive(serde::Serialize)]
pub enum Rating {
    Like,
    Dislike,
    None,
}

#[derive(serde::Serialize)]
struct RateQuery {
    id: String,
    rating: Rating,
}

#[derive(serde::Serialize)]
struct RefreshTokenQuery {
    client_id: String,
    client_secret: String,
    grant_type: String,
    refresh_token: String,
}

#[derive(Debug)]
pub struct YoutubeClient {
    client: Client,
    refresh_token: String,
    expiration_date: SystemTime,
}

impl YoutubeClient {
    async fn ensure_relevance(&mut self) {
        match self.expiration_date.elapsed() {
            // access_code has already expired
            Ok(_) => {
                self.refresh_tokens().await;
            }

            // access_code is not yet expired, check if its about to expire (in a minute)
            Err(negative_offset) => {
                if negative_offset.duration().as_secs() < 60 {
                    self.refresh_tokens().await;
                }
            }
        }
    }

    pub async fn rate_video(&mut self, url: &str, rating: Rating) -> reqwest::Response {
        self.ensure_relevance().await;

        let id = extract_video_id(url).unwrap();

        self.client
            .post(RATE_URL)
            .json(&RateQuery { id, rating })
            .send()
            .await
            .unwrap()
    }

    pub async fn new() -> Self {
        async fn read_data(filename: &'static str, mut path: PathBuf) -> String {
            path.push(filename);

            let mut fd = File::open(path).await.expect("failed to open file");

            let mut buf = String::new();
            fd.read_to_string(&mut buf)
                .await
                .expect("failed to read from file");

            buf
        }

        let base_path = expanduser("~/.config/fum").expect("failed to find fum config path");

        let access_token_handle = tokio::spawn(read_data("access_token", base_path.clone()));
        let refresh_token_handle = tokio::spawn(read_data("refresh_token", base_path.clone()));
        let expiration_date_handle =
            tokio::spawn(read_data("access_token_expiration_date", base_path));

        let expiration_date = expiration_date_handle.await.unwrap();
        let expiration_date: u64 = expiration_date
            .parse()
            .expect("failed to parse expiration date");
        let expiration_date = SystemTime::UNIX_EPOCH + Duration::from_secs(expiration_date);

        let access_token = access_token_handle.await.unwrap();
        let refresh_token = refresh_token_handle.await.unwrap();

        let header_val =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {access_token}")).unwrap();
        let mut header_map = reqwest::header::HeaderMap::new();
        header_map.insert("Authorization", header_val);

        let client = reqwest::ClientBuilder::new()
            .default_headers(header_map)
            .build()
            .expect("failed to create http client");

        Self {
            client,
            refresh_token,
            expiration_date,
        }
    }

    pub async fn refresh_tokens(&mut self) {
        let resp = self
            .client
            .post(TOKEN_URL)
            .json(&RefreshTokenQuery {
                client_id: CLIENT_ID.to_string(),
                client_secret: CLIENT_SECRET.to_string(),
                grant_type: "refresh_token".to_string(),
                refresh_token: self.refresh_token.clone(),
            })
            .send()
            .await
            .expect("failed to send refresh token request");

        let resp: ExchangeTokensResponse = resp
            .json()
            .await
            .expect("error while getting response for refreshing tokens");

        let access_token = resp.access_token;

        if let Some(refresh_token) = resp.refresh_token {
            self.refresh_token = refresh_token;
        }
        self.expiration_date = SystemTime::now() + Duration::from_secs(resp.expires_in as u64);

        let header_val =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {access_token}")).unwrap();
        let mut header_map = reqwest::header::HeaderMap::new();
        header_map.insert("Authorize", header_val);

        let client = reqwest::ClientBuilder::new()
            .default_headers(header_map)
            .build()
            .expect("failed to create http client");

        self.client = client;
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn test_rate_video() {
        let path = expanduser("~/.config/fum/access_token").unwrap();
        let mut fd = File::open(path).await.unwrap();

        let mut token = String::new();
        fd.read_to_string(&mut token).await.unwrap();

        let mut client = YoutubeClient::new().await;

        let response = client
            .rate_video(
                "https://music.youtube.com/watch?v=i0pfFewnYLw&list=RDAMVMi0pfFewnYLw",
                Rating::None,
            )
            .await;

        println!("{}", response.status());
    }

    #[tokio::test]
    async fn test_new_client_refresh() {
        let mut client = YoutubeClient::new().await;

        // println!("{client:#?}");

        client.refresh_tokens().await;
        // println!("{client:#?}");
    }
}
