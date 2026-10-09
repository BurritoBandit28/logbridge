use crate::discord;
use crate::util::GetSetWrapper;
use lazy_static::lazy_static;
use regex::Regex;
use serenity::all::{Http, ShardManager};
use serenity::gateway::ShardMessenger;
use std::collections::{HashMap, HashSet};
use std::process::ExitStatus;
use std::sync::Arc;
use std::sync::Mutex as VMutex;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::process::{ChildStdout, Command};
use tokio::sync::Mutex;

// maybe you can tell, but I use a mutex too much
lazy_static! {
    pub static ref INSTANCE: LogParser = LogParser::new();
    pub static ref chat_send_queue: Mutex<Vec<String>> = Mutex::new(vec![]);
    pub static ref start_logging: VMutex<GetSetWrapper<bool>> = VMutex::new(GetSetWrapper::new());
    pub static ref connected_player_count: VMutex<GetSetWrapper<i32>> =
        VMutex::new(GetSetWrapper::new());
}

pub struct LogParser {
    chat_regex: Regex,
    joined_regex: Regex,
    left_regex: Regex,
    advancement_regex: Regex,
    challenge_regex: Regex,
    death_regex: Regex,
    server_started: Regex,
    server_stopping: Regex,
    connected_players: VMutex<HashSet<String>>,
}

#[derive(Clone, Debug)]
pub enum MessageType {
    JOIN,
    LEAVE,
    DEATH,
    CHAT,
    ADVANCEMENT,
    CHALLENGE,
}

#[derive(Debug)]
pub struct ChatMessage {
    message_type: MessageType,
    message_text: String,
    player_name: String,
}

impl ChatMessage {
    pub fn get_player(&self) -> String {
        self.player_name.clone()
    }
    pub fn get_message_text(&self) -> String {
        self.message_text.clone()
    }
    pub fn get_message_type(&self) -> MessageType {
        self.message_type.clone()
    }
}

impl LogParser {
    fn new() -> Self {
        // old regex \[[0-9:]+\] \[[A-Za-z0-9/_\- ]+\]: <[A-Za-z0-9_-]+>

        // I have somehow kinda learnt regex now

        // maybe this is too much

        //idk
        let chat = Regex::new(r"^\[[^\]]+\]\s*\[Server thread/INFO\].+\s*<([^>]+)> (.+)").unwrap();
        let challenge =
            Regex::new(r"^\[[^\]]+\]\s*\[Server thread/INFO\].+\s*\<([A-Za-z-0-9_\-]+).*has completed the challenge ([\[A-Za-z 0-9_\-!.\]]+)").unwrap();
        let advancement =
            Regex::new(r"^\[[^\]]+\]\s*\[Server thread/INFO\].+\s*\<([A-Za-z-0-9_\-]+).*has made the advancement ([\[A-Za-z 0-9_\-!.\]]+)").unwrap();
        let joined_game = Regex::new(
            r"^\[[^\]]+\]\s*\[Server thread/INFO\].+\s([A-Za-z-0-9_\-]+) joined the game",
        )
        .unwrap();
        let left_the_game =
            Regex::new(r"^\[[^\]]+\]\s*\[Server thread/INFO\].+\s([A-Za-z-0-9_\-]+) left the game")
                .unwrap();
        let death =
            Regex::new(r"^\[[^\]]+\]\s*\[Server thread/INFO\](.+|):\s(\<([A-Za-z-0-9_\-]+).*)")
                .unwrap();
        let start_logging_indicator =
            Regex::new(r"^\[[^\]]+\]\s*\[Server thread/INFO\].+\sDone.+! For help, type").unwrap();
        let server_stop = Regex::new(
            r"^\[[^\]]+\]\s*\[Server thread/INFO\].+ (Stopping server|Stopping the server)",
        )
        .unwrap();

        Self {
            chat_regex: chat,
            joined_regex: joined_game,
            left_regex: left_the_game,
            advancement_regex: advancement,
            challenge_regex: challenge,
            death_regex: death,
            server_started: start_logging_indicator,
            server_stopping: server_stop,
            connected_players: VMutex::new(HashSet::new()),
        }
    }

    /// Returns the [ChatMessage] type containing information about the message, and the change in player count
    /// This method is designed to have its output passed to [discord::send_minecraft_embed()]
    pub fn try_parse_line(&self, line: String) -> Option<(ChatMessage, i32)> {
        let mut log = start_logging.lock().unwrap();
        if !log.get()
            && let Some(caps) = self.server_started.captures(&line)
        {
            log.set(true);
            return Some((
                ChatMessage {
                    message_type: MessageType::DEATH,
                    message_text: "Server Started".to_string(),
                    player_name: "MHF_Grass".to_string(),
                },
                0,
            ));
        }
        if log.get()
            && let Some(caps) = self.server_stopping.captures(&line)
        {
            let mut pc = 0;
            if let Ok(player_count) = connected_player_count.lock() {
                pc = player_count.get()
            }
            log.set(false);
            return Some((
                ChatMessage {
                    message_type: MessageType::DEATH,
                    message_text: "Server Stopped".to_string(),
                    player_name: "MHF_Grass".to_string(),
                },
                -pc,
            ));
        }
        if let Some(caps) = self.chat_regex.captures(&line) {
            if line
                .split(":")
                .collect::<Vec<&str>>()
                .get(3)
                .unwrap()
                .replace(" ", "")
                .starts_with("/")
            {
                return None;
            }

            let mut player_name = String::from(&caps[1]);
            let message_text = String::from(&caps[2]);
            let mut player_set = self.connected_players.lock().unwrap();
            if player_set.contains(&player_name) {
                return Some((
                    ChatMessage {
                        message_type: MessageType::CHAT,
                        message_text,
                        player_name,
                    },
                    0,
                ));
            }
        } else if let Some(caps) = self.joined_regex.captures(&line) {
            let player_name = String::from(&caps[1]);
            let mut player_set = self.connected_players.lock().unwrap();
            player_set.insert(player_name.clone());
            return Some((
                ChatMessage {
                    message_type: MessageType::JOIN,
                    message_text: format!("{} joined the game", player_name),
                    player_name,
                },
                1,
            ));
        } else if let Some(caps) = self.left_regex.captures(&line) {
            let player_name = String::from(&caps[1]);
            let mut player_set = self.connected_players.lock().unwrap();
            player_set.remove(&player_name.clone());
            return Some((
                ChatMessage {
                    message_type: MessageType::LEAVE,
                    message_text: format!("{} left the game", player_name),
                    player_name,
                },
                -1,
            ));
        } else if let Some(caps) = self.advancement_regex.captures(&line) {
            let player_name = String::from(&caps[1]);
            return Some((
                ChatMessage {
                    message_type: MessageType::ADVANCEMENT,
                    message_text: format!("{} has made the advancement {}", player_name, &caps[2]),
                    player_name,
                },
                0,
            ));
        } else if let Some(caps) = self.challenge_regex.captures(&line) {
            let player_name = String::from(&caps[1]);
            return Some((
                ChatMessage {
                    message_type: MessageType::CHALLENGE,
                    message_text: format!(
                        "{} has completed the challenge {}",
                        player_name, &caps[2]
                    ),
                    player_name,
                },
                0,
            ));
        } else if let Some(caps) = self.death_regex.captures(&line)
            && log.get()
            && !&caps[1].contains(":")
        {
            let player_name = String::from(&caps[3]);
            let mut player_set = self.connected_players.lock().unwrap();
            if player_set.contains(&player_name) {
                return Some((
                    ChatMessage {
                        message_type: MessageType::DEATH,
                        message_text: caps[2].to_string(),
                        player_name,
                    },
                    0,
                ));
            }
        }
        None
    }
}

pub async fn start_server(
    command: &mut Command,
    http: &Arc<Http>,
    shard_manager: Arc<ShardManager>,
) -> Result<ExitStatus, ()> {
    let mut handle = command.spawn().unwrap();
    let mut stdout = handle.stdout.take().ok_or("handle present").unwrap();
    let mut stdin = handle.stdin.take().ok_or("handle present").unwrap();
    let reader: TokioBufReader<ChildStdout> = TokioBufReader::new(stdout);
    let mut lines = reader.lines();

    tokio::spawn(async move {
        loop {
            if let Ok(mut queue_guard) = chat_send_queue.try_lock() {
                for message in queue_guard.drain(..) {
                    if let Err(e) = stdin.write_all(message.as_bytes()).await {
                        eprintln!("Failed to write to stdin: {}", e);
                        break;
                    }
                    if let Err(e) = stdin.flush().await {
                        eprintln!("Failed to flush stdin: {}", e);
                        break;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });

    while let Ok(Some(line)) = lines.next_line().await {
        let chat_message = INSTANCE.try_parse_line(line.clone());
        println!("{}", line);

        if let Some((msg, should_update_status)) = chat_message {
            if should_update_status != 0 {
                if let Ok(mut connected_players) = connected_player_count.lock() {
                    connected_players.add(should_update_status);
                }
            }
            // the send embed function triggers the status to change, as the bot detects its own message being sent
            // and uses that detection to update the status using the Context found in MessageHandler functions
            // its jank but hey it works
            discord::send_minecraft_embed(http, msg).await;
        }
    }

    let status = handle.wait().await.unwrap();
    if !status.success() {
        eprintln!("Process exited with status: {}", status);
    }
    println!("Minecraft Server Offline!");
    shard_manager.shutdown_all().await;
    Ok(status)
}
