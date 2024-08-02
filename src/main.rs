use clap::{Parser, ValueHint};
use notify::Watcher;
use std::fs;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use std::process::Child;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Files to watch
    #[arg(short = 'f', long = "files")]
    files: Vec<String>,
    /// File containing list of files to watch
    #[arg(short = 'l', long = "list")]
    list: Option<String>,
    /// Interval between two child process to start, in milliseconds
    #[arg(short = 'i', long = "interval", default_value = "1000")]
    interval: u64,
    /// Re-execute command on file change
    #[arg(short = 'r', long = "reexec")]
    reexec: bool,
    /// Kill previous child process when trigger again. Always true when reexec is enabled
    #[arg(short = 'k', long = "kill")]
    kill: bool,
    /// Do not print anything except errors
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,
    /// Command to run
    #[arg(required = true, num_args(1..), value_hint = ValueHint::CommandWithArguments, trailing_var_arg(true))]
    command: Vec<String>,
}

fn main() {
    let args = Args::parse();
    let files: Vec<String> = {
        if let Some(list) = args.list {
            let file = fs::File::open(list).expect("Failed to open file");
            let reader = BufReader::new(file);
            let files: Vec<String> = reader.lines().filter_map(|line| line.ok()).collect();
            files
        } else {
            args.files
        }
    };
    if files.len() == 0 {
        eprintln!("No files to watch");
        std::process::exit(1);
    }

    let interval = args.interval;

    let (tx, rx) = std::sync::mpsc::channel::<u64>();
    let tx_ = Arc::new(Mutex::new(tx));
    let mut watcher =
        notify::recommended_watcher(move |res: notify::Result<notify::event::Event>| {
            let tx = tx_.clone();
            match res {
                Ok(event) => match event.kind {
                    notify::EventKind::Modify(_) => {
                        let lock = tx.lock().unwrap();
                        _ = lock.send(1);
                    }
                    notify::EventKind::Any => (),
                    notify::EventKind::Access(_) => (),
                    notify::EventKind::Create(_) => (),
                    notify::EventKind::Remove(_) => (),
                    notify::EventKind::Other => (),
                },
                Err(e) => eprintln!("watch error: {:?}", e),
            }
        })
        .unwrap();
    for file in files {
        if !args.quiet {
            println!("Watching file: {}", file);
        }
        if let Err(e) = watcher.watch(Path::new(&file), notify::RecursiveMode::Recursive) {
            eprintln!("Failed to watch file: {}", e);
            std::process::exit(1);
        }
    }
    let mut last = std::time::Instant::now();
    let mut cmd = args.command.iter();
    let mut command = std::process::Command::new(cmd.next().unwrap());
    for arg in cmd {
        command.arg(arg);
    }
    let mut child: Option<Child> = if args.reexec {
        command
            .spawn()
            .inspect(|_| {
                if !args.quiet {
                    println!("Child process started");
                }
            })
            .inspect_err(|e| eprintln!("Failed to start command: {}", e))
            .ok()
    } else {
        None
    };
    loop {
        rx.recv().unwrap();
        let now = std::time::Instant::now();
        if now - last > Duration::from_millis(interval) {
            last = now;
            if !args.quiet {
                println!("Change detected");
            }
            if let Some(mut prev_child) = child.take() {
                if args.kill && args.reexec {
                    if prev_child
                        .kill()
                        .inspect_err(|e| eprintln!("Failed to kill child: {}", e))
                        .is_ok()
                        && prev_child
                            .wait()
                            .inspect_err(|e| eprintln!("Failed to finalize child: {}", e))
                            .is_ok()
                    {
                        if !args.quiet {
                            println!("Child process terminated");
                        }
                    } else {
                        eprintln!("Failed to kill child process, skipping re-execution");
                    }
                } else {
                    let exited = prev_child.try_wait().unwrap().is_none();
                    if !exited {
                        child = Some(prev_child);
                    }
                }
            }
            if child.is_none() {
                child = command
                    .spawn()
                    .inspect(|_| {
                        if !args.quiet {
                            println!("Child process started");
                        }
                    })
                    .inspect_err(|e| eprintln!("Failed to start command: {}", e))
                    .ok();
            }
        }
    }
}
