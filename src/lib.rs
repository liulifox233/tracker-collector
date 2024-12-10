use futures::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use tracing::info;
use tracing_subscriber::{
    fmt::{format::Pretty, time::UtcTime},
    prelude::*,
};
use tracing_web::{performance_layer, MakeConsoleWriter};
use worker::*;

#[derive(serde::Deserialize, Debug)]
struct Trackers {
    trackers: Vec<String>,
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
async fn fetch(_req: HttpRequest, _env: Env, _ctx: Context) -> Result<Response> {
    let trackers = get_trackers().await;

    let result = trackers.join(",");

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

    let trackers = trackers.join(",");

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

    let ws_stream = tokio_tungstenite_wasm::connect(aria2_url)
        .await
        .expect("Failed to connect to websocket");

    let (mut tx, mut rx) = ws_stream.split();

    info!("Connected to websocket");

    let task = async {
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

    tx.send(tokio_tungstenite_wasm::Message::text(pay_load.to_string()))
        .await
        .expect("Failed to send message");

    info!("Message sent!");

    task.await;
}

async fn get_trackers() -> Vec<String> {
    tracing::info!("Fetching trackers");
    let trackers: Trackers =
        serde_yaml::from_str(include_str!("../trackers.yml")).expect("Failed to parse trackers");
    tracing::info!("Trackers: {:#?}", trackers);
    let requests: Vec<Request> = trackers
        .trackers
        .iter()
        .map(|tracker| Request::new(tracker, Method::Get).expect("Failed to create request"))
        .collect();
    let trackers_vec = Arc::new(Mutex::new(Vec::new()));
    let mut tasks = Vec::new();
    for request in requests {
        let trackers_vec = trackers_vec.clone();
        let task = async move {
            let mut response = Fetch::Request(request).send().await.unwrap();
            let text = response.text().await.unwrap();
            match serde_json::from_str::<Trackers>(&text) {
                Ok(trackers) => {
                    trackers_vec.lock().unwrap().extend(trackers.trackers);
                }
                Err(_) => {
                    let trackers_text: Vec<String> =
                        text.split(",").map(|s| s.to_string()).collect();
                    trackers_vec.lock().unwrap().extend(trackers_text);
                }
            };
        };
        tasks.push(task);
    }

    futures::future::join_all(tasks).await;

    let trackers = trackers_vec.lock().unwrap().clone();
    trackers
}
