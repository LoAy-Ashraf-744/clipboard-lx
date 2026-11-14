use std::env;
use std::fs;
use std::io::Read;
use std::process;

use clip_core::ipc::{send_request, Request, Response};
use clip_core::model::{format_byte_size, format_relative_time, now_millis, EntryKind};
use clip_core::paths::default_socket_path;

fn usage() -> ! {
    eprintln!(
        "Usage: clip-ctl <command> [args]

Commands:
  status                  Show daemon status and entry count
  list [--limit N]        List clipboard history
  search <query>          Search clipboard history
  add-text <text|--stdin> Add a text entry
  add-image <path|--stdin> Add a PNG image entry
  select <id>             Restore entry onto clipboard
  delete <id>             Delete an entry
  pin <id>                Toggle pin status on an entry
  clear                   Delete all unpinned entries
  pause                   Pause clipboard recording
  resume                  Resume clipboard recording
  notify-change           Trigger clipboard capture from active selection"
    );
    process::exit(2);
}

fn print_entries(entries: &[clip_core::model::ClipboardEntry]) {
    let now = now_millis();
    if entries.is_empty() {
        println!("(empty)");
        return;
    }
    for e in entries {
        let pin = if e.pinned { "📌" } else { "  " };
        let kind = match e.kind {
            EntryKind::Text => "TEXT",
            EntryKind::Image => "IMG ",
        };
        let preview = e.text.replace(['\n', '\r', '\t'], " ");
        let preview_trunc: String = preview.chars().take(72).collect();
        let meta = match e.kind {
            EntryKind::Text => format_byte_size(e.byte_size),
            EntryKind::Image => format!("{}×{}", e.width, e.height),
        };
        println!(
            "{} [{:>4}] {}  {}  ({})  {}",
            pin,
            e.id,
            kind,
            preview_trunc,
            meta,
            format_relative_time(e.updated_at, now)
        );
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        usage();
    }

    let sock = default_socket_path();
    let req = match args[0].as_str() {
        "status" => Request::Status,
        "list" => {
            let mut limit = 50u32;
            if args.len() >= 3 && args[1] == "--limit" {
                limit = args[2].parse().unwrap_or(50);
            } else if args.len() >= 2 {
                if let Ok(n) = args[1].parse::<u32>() {
                    limit = n;
                }
            }
            Request::List {
                limit,
                include_images: false,
            }
        }
        "search" => {
            if args.len() < 2 {
                usage();
            }
            let query = args[1..].join(" ");
            Request::Search {
                query,
                limit: 50,
                include_images: false,
            }
        }
        "add-text" => {
            if args.len() < 2 {
                usage();
            }
            let text = if args[1] == "--stdin" {
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf).unwrap_or(0);
                buf
            } else {
                args[1..].join(" ")
            };
            Request::AddText { text }
        }
        "add-image" => {
            if args.len() < 2 {
                usage();
            }
            let png = if args[1] == "--stdin" {
                let mut buf = Vec::new();
                std::io::stdin().read_to_end(&mut buf).unwrap_or(0);
                buf
            } else {
                fs::read(&args[1]).unwrap_or_else(|e| {
                    eprintln!("Failed to read {}: {}", args[1], e);
                    process::exit(1);
                })
            };
            Request::AddImage { png }
        }
        "select" => {
            let id: i64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or_else(|| usage());
            Request::Select { id }
        }
        "delete" => {
            let id: i64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or_else(|| usage());
            Request::Delete { id }
        }
        "pin" => {
            let id: i64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or_else(|| usage());
            Request::TogglePin { id }
        }
        "clear" => Request::Clear,
        "pause" => Request::SetPaused { paused: true },
        "resume" => Request::SetPaused { paused: false },
        "ping" => Request::Ping,
        "notify-change" | "sync" => Request::Sync,
        _ => usage(),
    };

    match send_request(&sock, &req) {
        Ok(resp) => match resp {
            Response::Pong => println!("pong"),
            Response::Entries(entries) => print_entries(&entries),
            Response::Entry(Some(e)) => println!("Selected [{}] {}", e.id, e.text),
            Response::Entry(None) => println!("Not found"),
            Response::Added(id) => println!("Added entry {}", id),
            Response::Bool(b) => println!("{}", b),
            Response::Cleared(n) => println!("Cleared {} unpinned entries", n),
            Response::StatusInfo {
                count,
                paused,
                db_path,
            } => {
                println!(
                    "clipd running | entries: {} | paused: {} | db: {}",
                    count, paused, db_path
                );
            }
            Response::Error(msg) => {
                eprintln!("Error: {}", msg);
                process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("{}", e);
            process::exit(1);
        }
    }
}
