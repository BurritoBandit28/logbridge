mod discord;
mod minecraft;
mod util;

use crate::discord::{channel_id_val, server_ip, should_use_server_icon};
use crate::minecraft::{chat_send_queue, start_server};
use discord::MessageHandler;
use serenity::Client;
use serenity::all::{EventHandler, GatewayIntents};
use std::io::{BufRead, Read, Write};
use std::ops::Deref;
use std::process::{Stdio, exit};
use std::sync::Arc;
use std::{env, thread};
use std::path::Path;
use property_rs::Properties;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::join;
use tokio::process::Command;
use tokio_util::io::simplex::new;
use tokio_util::sync::CancellationToken;

async fn handle_console_input(token: CancellationToken) {
    let stdin = io::stdin();
    let mut reader = TokioBufReader::new(stdin);

    'detection_loop: loop {
        if token.is_cancelled() {
            break 'detection_loop;
        }
        let mut buf = String::new();

        match reader.read_line(&mut buf).await {
            Ok(0) => {
                println!("Console input closed.");
                break;
            }
            Ok(_) => {
                if let Ok(mut queue_guard) = chat_send_queue.try_lock() {
                    queue_guard.push(buf);
                }
            }
            Err(e) => {
                eprintln!("Failed to read from console: {}", e);
                break;
            }
        }
    }
}

#[tokio::main]
async fn main() {

    let property_path = Path::new("./logbridge.properties");

    let mut properties : Properties;

    if !property_path.exists() {
        properties = Properties::new();
        properties.add_set("bot_token","");
        properties.add_set("channel_id","");
        properties.add_set("use_server_icon","false");
        properties.add_set("server_address","");
        properties.save_to("./logbridge.properties", Some("Logbridge properties")).expect("Failed to create properties file");
    }
    else {
        properties = Properties::from_file("./logbridge.properties").expect("Failed to read properties file");
    }

    let token = properties.get("bot_token").expect("Field \"bot_token\" is missing");
    channel_id_val
        .lock()
        .await
        .set(properties.get("channel_id").expect("Field \"channel_id\" is missing").parse::<u64>().unwrap());

    should_use_server_icon
        .lock()
        .await
        .set(properties.get("use_server_icon").expect("Field \"use_server_icon\" is missing").parse::<bool>().unwrap());

    server_ip
        .lock()
        .await
        .set(properties.get("server_address").expect("Field \"server_address\" is missing"));

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("No launch command specified");
        return;
    }

    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;
    let mut client = Client::builder(token, intents)
        .event_handler(MessageHandler)
        .await
        .expect("Err creating client, has the token been provided?");

    let http = Arc::clone(&client.http);

    let mut temp : String = "".to_string();
    for i in 2..args.len() {
        temp = format!("{}{} ",temp, args[i])
    }
    let launch_args = temp.trim();
    let launch_program = &args[1];


    println!("{}",launch_program);
    println!("{}", launch_args);

    //let mut command = Command::new(format!("./{}",minecraft_server_launch_command));
    let mut command: Command;
    command = Command::new(launch_program);
    command.args(launch_args.split(" "));


    command.stdout(Stdio::piped());
    command.stdin(Stdio::piped());

    let token = CancellationToken::new();

    let read_input_handle = tokio::spawn(handle_console_input(token.clone()));

    let shard_manager = client.shard_manager.clone();

    let (server_exit_status, _) = join!(
        start_server(&mut command, &http, shard_manager),
        client.start()
    );

    if server_exit_status.is_ok() {
        println!("Server Stopped Successfully");
    } else {
        println!("Server Stopped Unsuccessfully");
    }
    println!("press enter to close");
    token.cancel();
    read_input_handle.await.unwrap();
}
