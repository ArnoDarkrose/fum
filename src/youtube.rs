use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{self, Duration, SystemTime};

use actix_web::{get, web, App, HttpServer, Responder};
use expanduser::expanduser;
use reqwest::Client;

use tokio::io::AsyncWriteExt;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::task::JoinSet;

use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_tracing::TracingMiddleware;

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
    sender: Mutex<Option<oneshot::Sender<JoinHandle<()>>>>,
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

    if !fs::exists(&base_path).expect("could not check path existance") {
        fs::create_dir_all(&base_path).expect("could not create directory");
    }

    fn save_data<T: std::fmt::Display + Send + Sync + 'static>(
        set: &mut JoinSet<()>,
        data: T,
        filename: &'static str,
        base_path: std::path::PathBuf,
    ) {
        let mut path = base_path.clone();
        path.push(filename);

        set.spawn(async move {
            let mut fd = tokio::fs::File::create(&path)
                .await
                .expect("fum config dir not found");

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

async fn start_server(sender: Mutex<Option<oneshot::Sender<JoinHandle<()>>>>) {
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

#[derive(serde::Serialize, Debug)]
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
pub struct YouTubeClient {
    client: Client,
    refresh_token: String,
    expiration_date: SystemTime,
}

#[derive(Debug)]
pub enum YouTubeAction {
    RateVideo {
        url: String,
        sender: oneshot::Sender<reqwest::Response>,
        rating: Rating,
    },
}

impl YouTubeClient {
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

    fn new() -> Self {
        fn read_data(filename: &'static str, mut path: PathBuf) -> String {
            path.push(filename);

            if !fs::exists(&path).expect("failed to look up path existance") {
                panic!("You need to authorize first");
            }

            let mut fd = std::fs::File::open(path).expect("failed to open file");

            let mut buf = String::new();
            fd.read_to_string(&mut buf)
                .expect("failed to read from file");

            buf
        }

        let base_path = expanduser("~/.config/fum").expect("failed to find fum config path");

        if !fs::exists(&base_path).expect("failed to check path existance") {
            fs::create_dir_all(&base_path).expect("failed to create fum config directory");
        }

        let expiration_date = read_data("access_token_expiration_date", base_path.clone());
        let expiration_date: u64 = expiration_date
            .parse()
            .expect("failed to parse expiration date");
        let expiration_date = SystemTime::UNIX_EPOCH + Duration::from_secs(expiration_date);

        let access_token = read_data("access_token", base_path.clone());
        let refresh_token = read_data("refresh_token", base_path);

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

    pub fn get_handle() -> mpsc::Sender<YouTubeAction> {
        let mut client = Self::new();

        let (sender, mut receiver) = tokio::sync::mpsc::channel(10);

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("failed to initialize tokio runtime");

            rt.block_on(async move {
                while let Some(msg) = receiver.recv().await {
                    match msg {
                        YouTubeAction::RateVideo {
                            url,
                            sender: resp_sender,
                            rating,
                        } => {
                            let resp = client.rate_video(&url, rating).await;

                            resp_sender
                                .send(resp)
                                .expect("failed to send youtube server response through channel");
                        }
                        _ => {
                            panic!("unexpected variant of YouTubeAction")
                        }
                    }
                }
            })
        });

        sender
    }

    async fn save_data(access_token: String, refresh_token: String, expiration_date: u64) {
        fn save_on_disk<T: std::fmt::Display + Send + Sync + 'static>(
            set: &mut JoinSet<()>,
            data: T,
            filename: &'static str,
            base_path: std::path::PathBuf,
        ) {
            let mut path = base_path.clone();
            path.push(filename);

            set.spawn(async move {
                let mut fd = tokio::fs::File::create(&path)
                    .await
                    .expect("fum config dir not found");

                fd.write_all(format!("{data}").as_bytes())
                    .await
                    .expect("failed to write to file");
            });
        }

        let mut set = JoinSet::new();

        let base_path = expanduser("~/.config/fum").expect("fum config dir not found");

        save_on_disk(&mut set, access_token, "access_token", base_path.clone());

        save_on_disk(&mut set, refresh_token, "refresh_token", base_path.clone());

        save_on_disk(
            &mut set,
            expiration_date,
            "access_token_expiration_date",
            base_path,
        );

        set.join_all().await;
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

        tokio::spawn(YouTubeClient::save_data(
            access_token,
            self.refresh_token.clone(),
            self.expiration_date
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        ))
        .await
        .unwrap();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::fs::File;

    #[test]
    fn test_rate_video() {
        let path = expanduser("~/.config/fum/access_token").unwrap();
        let mut fd = File::open(path).unwrap();

        let mut token = String::new();
        fd.read_to_string(&mut token).unwrap();

        let client_handle = YouTubeClient::get_handle();

        let (sender, receiver) = oneshot::channel();
        client_handle
            .blocking_send(YouTubeAction::RateVideo {
                url: "https://music.youtube.com/watch?v=i0pfFewnYLw&list=RDAMVMi0pfFewnYLw"
                    .to_string(),
                sender,
                rating: Rating::Like,
            })
            .unwrap();

        let response = receiver.blocking_recv().unwrap();
        println!("{}", response.status());
    }

    #[tokio::test]
    async fn test_refresh_token() {
        let mut client = YouTubeClient::new();

        println!("{client:#?}");

        client.refresh_tokens().await;

        println!("{client:#?}");
    }
}
