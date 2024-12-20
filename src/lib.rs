use dashmap::DashSet;
use futures::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use tracing::info;
use tracing_subscriber::{
    fmt::{format::Pretty, time::UtcTime},
    prelude::*,
};
use tracing_web::{performance_layer, MakeConsoleWriter};
use wasm_bindgen::JsValue;
use worker::*;

#[derive(serde::Deserialize, Debug)]
struct Trackers {
    trackers: DashSet<String>,
}

#[event(start)]
fn start() {
    console_error_panic_hook::set_once();
    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_ansi(false) // Only partially supported across JavaScript runtimes
        .with_timer(UtcTime::rfc_3339()) // std::time is not available in browsers
        .with_writer(MakeConsoleWriter); // write events to the console
    let perf_layer = performance_layer().with_details_from_fields(Pretty::default());
    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(perf_layer)
        .init();
}

#[event(fetch)]
async fn fetch(req: HttpRequest, _env: Env, _ctx: Context) -> Result<Response> {
    let spilter = if req.uri().path() == "/" { "," } else { "\n\n" };
    let trackers = get_trackers().await;

    let result = Vec::from(
        trackers
            .iter()
            .map(|tracker| tracker.clone())
            .collect::<Vec<String>>(),
    )
    .join(&spilter);

    Response::ok(result)
}

#[event(scheduled)]
async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) {
    info!("Scheduled event");
    let aria2_url = env
        .secret("ARIA2_URL")
        .expect("ARIA2_URL secret not found")
        .to_string();
    let secret_key = env
        .secret("SECRET_KEY")
        .expect("SECRET_KEY secret not found")
        .to_string();

    let trackers = get_trackers().await;

    info!("Total trackers: {}", trackers.len());

    let trackers = Vec::from(
        trackers
            .iter()
            .map(|tracker| tracker.clone())
            .collect::<Vec<String>>(),
    )
    .join(",");

    let pay_load = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "aria2.changeGlobalOption",
        "id": "cron",
        "params": [
            format!("token:{}", secret_key),
            {
                "bt-tracker": trackers
            }
        ]
    });
    if aria2_url.starts_with("http") {
        change_global_option_http(aria2_url, pay_load).await;
    } else {
        change_global_option_ws(aria2_url, pay_load).await;
    }
}

async fn get_trackers() -> DashSet<String> {
    tracing::info!("Fetching trackers");
    let trackers: Trackers =
        serde_yaml::from_str(include_str!("../trackers.yml")).expect("Failed to parse trackers");
    tracing::info!("Trackers: {:#?}", trackers);
    let (trackers_vec, request): (Vec<String>, Vec<String>) = trackers
        .trackers
        .into_iter()
        .partition(|tracker| tracker.ends_with("announce"));
    let trackers_set: DashSet<String> = trackers_vec.into_iter().collect();
    let requests: Vec<Request> = request
        .iter()
        .map(|tracker| Request::new(&tracker, Method::Get).expect("Failed to create request"))
        .collect();
    let trackers_set = Arc::new(Mutex::new(trackers_set));
    let mut tasks = Vec::new();
    requests.into_iter().for_each(|request| {
        let trackers_set = trackers_set.clone();
        let task = async move {
            let mut response = Fetch::Request(request).send().await.unwrap();
            let text = response.text().await.unwrap();
            let trackers = parse_tracker(&text);
            trackers_set.lock().unwrap().extend(trackers);
        };
        tasks.push(task);
    });

    futures::future::join_all(tasks).await;

    let trackers = trackers_set.lock().unwrap().clone();
    trackers
}

async fn change_global_option_http(aria2_url: String, pay_load: serde_json::Value) {
    let mut headers = Headers::new();
    headers.set("Content-Type", "application/json").unwrap();
    let request = Request::new_with_init(
        &aria2_url,
        &RequestInit {
            method: Method::Post,
            headers,
            body: Some(JsValue::from(pay_load.to_string())),
            ..RequestInit::default()
        },
    )
    .expect("Failed to create request");

    let response = Fetch::Request(request)
        .send()
        .await
        .expect("Failed to send request")
        .json::<serde_json::Value>()
        .await
        .unwrap();
    match response.get("result") {
        Some(result) => {
            info!("Response: {}", result.to_string());
        }
        None => {
            info!("Error: {:#?}", response["error"]);
        }
    }
}

async fn change_global_option_ws(aria2_url: String, pay_load: serde_json::Value) {
    let ws_stream = tokio_tungstenite_wasm::connect(aria2_url)
        .await
        .expect("Failed to connect to websocket");

    let (mut tx, mut rx) = ws_stream.split();

    info!("Connected to websocket");

    let receive_task = async move {
        while let Some(msg) = rx.next().await {
            let msg = msg.expect("Failed to receive message");
            let msg = serde_json::from_str::<serde_json::Value>(&msg.to_string())
                .expect("Failed to parse message");
            if Some("cron") == msg.get("id").and_then(|v| v.as_str()) {
                match msg.get("result") {
                    Some(result) => {
                        info!("Result: {:#?}", result);
                    }
                    None => {
                        info!("Error: {:#?}", msg["error"]);
                    }
                }
                break;
            }
        }
    };

    let send_task = async move {
        tx.send(tokio_tungstenite_wasm::Message::text(pay_load.to_string()))
            .await
            .expect("Failed to send message");
        info!("Message sent!");
    };

    futures::future::join(receive_task, send_task).await;
}

fn parse_tracker(trackers_list: &str) -> DashSet<String> {
    if let Ok(trackers) = serde_json::from_str::<Trackers>(&trackers_list) {
        return trackers.trackers;
    };
    if trackers_list.contains(",") {
        return trackers_list
            .split(",")
            .map(|tracker| tracker.to_string())
            .collect();
    }
    if trackers_list.contains("\n\n") {
        return trackers_list
            .split("\n\n")
            .map(|tracker| tracker.to_string())
            .collect();
    }
    panic!("Invalid tracker list format");
}
