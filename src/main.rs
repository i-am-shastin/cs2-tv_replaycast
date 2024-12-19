use std::{collections::HashMap, error::Error, fs, io, sync::Arc};
use chrono::Local;
use reqwest::{header, ClientBuilder, RequestBuilder};
use tokio::{sync::Mutex, task, time::{interval, Duration, Instant, MissedTickBehavior}};
use teloxide::{prelude::*, types::Recipient};
use serde::Deserialize;

const BUFFER_SIZE_IN_FRAGMENTS: i32 = 50;
const HTTP_CLIENT_TIMEOUT_IN_SECONDS: u64 = 5;
const FRAGMENT_RETRY_COUNT: i32 = 5;

const RETRY_DELAY_IN_SECONDS: u64 = 15;
const CSTV_STOPPED_RETRY_DELAY_IN_SECONDS: u64 = 60;
const CONNECTION_RESET_RETRY_DELAY_IN_SECONDS: u64 = 3;

const TELEGRAM_BOT_RECIPIENT: Recipient = Recipient::Id(ChatId(-4625355477));
const TELEGRAM_MAP_CHANGE_NOTIFICATION_DELAY: u64 = 60 * 5;

const FALLBACK_URL_DELAY_SECONDS: u64 = 40;
const FALLBACK_RETRY_COUNT: i32 = 2;

const MAX_CONSECUTIVE_ERRORS: i32 = 5;

#[derive(Deserialize)]
struct StreamConfig {
    url: String,
    fallback_url: String,
    directory: String,
}

#[tokio::main]
async fn main() {
    if dotenvy::dotenv().is_err() {
        log("Failed to load environment variables".to_string());
    }

    let config = match fs::read_to_string("stream.json") {
        Ok(content) => match serde_json::from_str::<StreamConfig>(&content) {
            Ok(config) => config,
            Err(e) => {
                log(format!("Failed to parse stream.json: {e}"));
                return;
            }
        },
        Err(e) => {
            log(format!("Failed to read stream.json: {e}"));
            return;
        }
    };

    let url = config.url;
    let stream_path = config.directory;

    log(format!("Program started. URL is {url}, stream path is {stream_path}"));
    try_clear_stream_folder(&stream_path);

    let client = ClientBuilder::new()
        .timeout(Duration::from_secs(HTTP_CLIENT_TIMEOUT_IN_SECONDS))
        .build().expect("Creating client");

    let mut headers = header::HeaderMap::new();
    headers.insert("Connection", header::HeaderValue::from_static("close"));
    let fallback_client = ClientBuilder::new()
        .default_headers(headers)
        .http1_only()
        .timeout(Duration::from_secs(HTTP_CLIENT_TIMEOUT_IN_SECONDS))
        .build().expect("Creating fallback client");

    let bot = Bot::from_env();
    let _ = bot.send_message(TELEGRAM_BOT_RECIPIENT, "Здарова пацаны! Нафига я на этот шоуматч пошёл, лучше бы с вами посидел. Пробую коннектиться к серверу").await;

    let mut pause_between_tries = interval(Duration::from_secs(RETRY_DELAY_IN_SECONDS));
    pause_between_tries.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let good_fragments_loaded = Arc::new(Mutex::new(0));
    let mut last_sync_fragment = -0xFFFF;
    let mut last_sync_fail_time: Option<Instant> = None;

    loop {
        pause_between_tries.tick().await;

        log("Trying to load stream sync data".to_string());
        let sync_response = match client.get(format!("{url}/sync")).send().await {
            Ok(r) => r,
            Err(e) => {
                log(format!("Can't sync: {e:?}"));
                continue;
            },
        };
        if sync_response.status() != reqwest::StatusCode::OK {
            log(sync_response.status().to_string());
            continue;
        }
        let sync_data = match sync_response.json::<HashMap<String, i32>>().await {
            Ok(data) => data,
            Err(e) => {
                log(format!("Can't parse JSON: {e:?}"));
                continue;
            },
        };

        // Sync data is successfully parsed, can use values from it
        let mut current_fragment = match sync_data.get("fragment") {
            Some(f) => *f,
            None => {
                log("Can't get current fragment from JSON".to_string());
                continue;
            },
        };
        let signup_fragment = match sync_data.get("signup_fragment") {
            Some(f) => f,
            None => {
                log("Can't get signup fragment from JSON".to_string());
                continue;
            },
        };

        if current_fragment == last_sync_fragment {
            log(format!("Seems like CSTV server is stopped, awaiting {CSTV_STOPPED_RETRY_DELAY_IN_SECONDS} second(s) before resync"));
            
            if last_sync_fail_time.is_none() || last_sync_fail_time.unwrap().elapsed().as_secs() > TELEGRAM_MAP_CHANGE_NOTIFICATION_DELAY {
                try_clear_stream_folder(&stream_path);
                let _ = bot.send_message(TELEGRAM_BOT_RECIPIENT, "Видимо на сервере ещё старая карта висит, буду проверять раз в минуту").await;
                last_sync_fail_time = Some(Instant::now());
            }
            
            tokio::time::sleep(Duration::from_secs(CSTV_STOPPED_RETRY_DELAY_IN_SECONDS)).await;
            continue;
        }
        last_sync_fragment = current_fragment;

        // Weird shit... Happens on map change
        if current_fragment < 0 {
            let _ = bot.send_message(TELEGRAM_BOT_RECIPIENT, "Походу карту сменили, подождём немного").await;
            log(format!("Fragment {current_fragment} is unavailable! Awaiting..."));
            tokio::time::sleep(Duration::from_secs(current_fragment.unsigned_abs().into())).await;
            try_clear_stream_folder(&stream_path);
            continue;
        };

        log("Trying to get stream start data".to_string());
        if fs::create_dir_all(format!("{stream_path}/{signup_fragment}")).is_err() {
            log("Can't create output folder".to_string());
            continue;
        }
        if fs::write(format!("{stream_path}/sync"), serde_json::to_string(&sync_data).unwrap()).is_err() {
            log("Can't write sync data".to_string());
            continue;
        }
        if let Err(e) = save(client.get(format!("{url}/{signup_fragment}/start")), format!("{stream_path}/{signup_fragment}/start")).await {
            log(format!("START fragment #{signup_fragment}: {e}"));
            continue;
        }

        let interval_in_seconds = sync_data.get("keyframe_interval").map_or(3, |i| *i as u64);
        let mut keyframe_interval = interval(Duration::from_secs(interval_in_seconds));
        log(format!("Sync data loaded, now receiving data packets each {interval_in_seconds} second(s)"));
        log("-".repeat(30));

        let consecutive_errors = Arc::new(Mutex::new(0));
        let mute_bot = Arc::new(Mutex::new(false));
        *good_fragments_loaded.lock().await = 0;
        loop {
            keyframe_interval.tick().await;

            if *good_fragments_loaded.lock().await == BUFFER_SIZE_IN_FRAGMENTS {
                log(format!("[READY] {BUFFER_SIZE_IN_FRAGMENTS} sec buffer is full"));
                let _ = bot.send_message(TELEGRAM_BOT_RECIPIENT, format!("Пробуйте подключаться, {BUFFER_SIZE_IN_FRAGMENTS}-секундный буфер готов")).await;
            }

            let task_client = client.clone();
            let task_fallback_client = fallback_client.clone();
            let task_consecutive_errors = Arc::clone(&consecutive_errors);
            let task_good_fragments_loaded = Arc::clone(&good_fragments_loaded);
            let task_url = format!("{url}/{current_fragment}");
            let task_fallback_url = format!("{}/{current_fragment}", config.fallback_url);
            let task_path = format!("{stream_path}/{current_fragment}");
            let task_bot = bot.clone();
            let task_mute_bot = Arc::clone(&mute_bot);

            task::spawn(async move {
                if fs::create_dir_all(&task_path).is_err() {
                    log(format!("Unable to create dir {task_path}"));
                    *task_consecutive_errors.lock().await += 1;
                    return;
                }

                let mut try_count = 0;
                let first_try_time = Instant::now();
                
                // Saving delta fragment from main URL - try 5 times then break & use fallback
                while try_count < FRAGMENT_RETRY_COUNT {
                    try_count += 1;
                    match save(task_client.get(format!("{task_url}/delta")), format!("{task_path}/delta")).await {
                        Ok(_) => {
                            *task_good_fragments_loaded.lock().await += 1;
                            try_count = 0;
                            break;
                        },
                        Err(e) => log(format!("DELTA fragment #{current_fragment}: {e} at try #{try_count}")),
                    };
                }

                // If main URL failed, wait until 40 seconds have passed since first try
                if try_count == FRAGMENT_RETRY_COUNT {
                    let elapsed = first_try_time.elapsed().as_secs();
                    if elapsed < FALLBACK_URL_DELAY_SECONDS {
                        let wait_time = FALLBACK_URL_DELAY_SECONDS - elapsed;
                        log(format!("Waiting {wait_time}s before trying fallback URL"));
                        tokio::time::sleep(Duration::from_secs(wait_time)).await;
                    }

                    // Try fallback URL
                    try_count = 0;
                    while try_count < FALLBACK_RETRY_COUNT {
                        try_count += 1;
                        match save(task_fallback_client.get(format!("{task_fallback_url}/delta")), format!("{task_path}/delta")).await {
                            Ok(_) => {
                                *task_good_fragments_loaded.lock().await += 1;
                                break;
                            },
                            Err(e) => {
                                if try_count == FALLBACK_RETRY_COUNT && !(*task_mute_bot.lock().await) {
                                    log(format!("[ERROR] DELTA fragment #{current_fragment}: fallback client failed: {e}"));
                                    let _ = task_bot.send_message(TELEGRAM_BOT_RECIPIENT, format!("Обрыв связи на пакете #{current_fragment}, надо будет делать реконнект")).await;
                                    *task_consecutive_errors.lock().await += 1;
                                    return;
                                }
                            },
                        };
                    }
                }

                // Saving full fragment - try 5 times then just give up and hope for the best
                try_count = 0;
                while try_count < FRAGMENT_RETRY_COUNT {
                    try_count += 1;
                    if save(task_client.get(format!("{task_url}/full")), format!("{task_path}/full")).await.is_ok() {
                        break;
                    } else if try_count < FRAGMENT_RETRY_COUNT {
                        continue;
                    }
                    log(format!("FULL fragment #{current_fragment} skipped after {try_count} tries"));
                }

                // If we got here successfully, reset error counter
                *task_consecutive_errors.lock().await = 0;
            });

            // Check error count and break if too many errors
            if *consecutive_errors.lock().await >= MAX_CONSECUTIVE_ERRORS {
                log("[ERROR] Too many consecutive errors, resyncing...".to_string());
                let _ = bot.send_message(TELEGRAM_BOT_RECIPIENT, format!("Бля, у нас {MAX_CONSECUTIVE_ERRORS} ошибок подряд. Пробую синхронизироваться заново")).await;
                *consecutive_errors.lock().await = 0;
                *mute_bot.lock().await = true;
                // *good_fragments_loaded.lock().await = 0;
                break;
            }

            // Go to next fragment if everything is ok
            if fs::write(format!("{stream_path}/current"), current_fragment.to_string()).is_err() {
                log("Unable to save current fragment number".to_string());
            }
            current_fragment += 1;
        }
    }
}

fn log(message: String) {
    let current_time = Local::now().format("%H:%M:%S").to_string();
    println!("[{current_time}] {message}");
}

async fn save(req: RequestBuilder, path: String) -> Result<(), String> {
    let response = match req.send().await {
        Ok(response) => {
            if response.status() != reqwest::StatusCode::OK {
                return Err(format!("incorrect status ({})", response.status()))
            }
            response
        },
        Err(e) => {
            if is_connection_reset(&e) {
                tokio::time::sleep(Duration::from_secs(CONNECTION_RESET_RETRY_DELAY_IN_SECONDS)).await;
            }
            return Err(format!("request failed ({e:?})"));
        },
    };

    let bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            if is_connection_reset(&e) {
                tokio::time::sleep(Duration::from_secs(CONNECTION_RESET_RETRY_DELAY_IN_SECONDS)).await;
            }
            return Err(format!("unable to read body bytes ({e:?})"));
        },
    };

    fs::write(path, bytes).map_err(|_| "writing file".to_string())
}

fn is_connection_reset(e: &reqwest::Error) -> bool {
    let mut current_error: &(dyn Error) = e;
    loop {
        if let Some(io_err) = current_error.downcast_ref::<io::Error>() {
            if io_err.kind() == io::ErrorKind::ConnectionReset {
                return true;
            }
        }

        match current_error.source() {
            Some(err) => current_error = err,
            None => return false,
        }
    }
}

fn try_clear_stream_folder(stream_path: &str) {
    if let Ok(true) = fs::exists(stream_path) {
        if fs::remove_dir_all(stream_path).is_err() {
            log("Error clearing existing stream folder".to_string());
        } else {
            log("Existing stream folder emptied".to_string());
        }
    }
}
