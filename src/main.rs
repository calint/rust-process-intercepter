use std::env;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::unix::io::AsRawFd;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const SLEEP_WHEN_NO_INPUT: u64 = 10;

fn main() -> io::Result<()> {
    // get command arguments
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <command> [args...]", args[0]);
        return Ok(());
    }

    // extract command and arguments
    let command = &args[1];
    let command_args = if args.len() > 2 {
        args[2..].to_vec()
    } else {
        vec![]
    };

    // open log file
    let mut log_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("input_log.bin")?;

    // spawn child process
    let mut child = Command::new(command)
        .args(&command_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    // take stdin to child
    let mut child_stdin = child.stdin.take().expect("stdin");

    // setup terminal for non-blocking, no echo
    setup_terminal();

    // create a channel for forwarding input
    let (tx, rx) = mpsc::channel();

    // thread for reading raw binary input and translating to serial
    thread::spawn(move || {
        loop {
            // translate from terminal to serial
            match get_byte_non_blocking() {
                -1 => {
                    thread::sleep(Duration::from_millis(SLEEP_WHEN_NO_INPUT));
                    continue;
                }
                0x0a => tx.send(0x0d).expect("send"), // carriage return
                0x08 => tx.send(0x7f).expect("send"), // backspace
                byte => tx.send(byte as u8).expect("send"),
            };
        }
    });

    for byte in rx {
        // log to binary file
        log_file.write_all(&[byte])?;
        log_file.flush()?;

        // forward to child
        child_stdin.write_all(&[byte])?;
        child_stdin.flush()?;
    }

    // wait for the child process to complete
    let status = child.wait()?;
    println!("\nProcess exited with status: {}", status);

    Ok(())
}

fn setup_terminal() {
    let stdin = io::stdin();
    let fd = stdin.as_raw_fd();

    // Save original terminal settings
    let mut saved_termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut saved_termios) } != 0 {
        panic!("Failed to get terminal attributes");
    }

    // Configure new terminal settings
    let mut newt = saved_termios;
    newt.c_lflag &= !(libc::ICANON | libc::ECHO);
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &newt) } != 0 {
        panic!("Failed to set terminal attributes");
    }

    // Set non-blocking mode
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags == -1 {
        panic!("Failed to get file flags");
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        panic!("Failed to set non-blocking mode");
    }

    // Register cleanup function
    let cleanup = move || unsafe {
        if libc::tcsetattr(fd, libc::TCSANOW, &saved_termios) != 0 {
            eprintln!("Failed to restore terminal attributes");
        }
        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        if flags != -1 || libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) == -1 {
            eprintln!("Failed to restore blocking mode");
        }
    };

    // Ensure cleanup runs on thread exit
    let _cleanup_guard = std::panic::catch_unwind(|| cleanup);
}

fn get_byte_non_blocking() -> i32 {
    unsafe { libc::getchar() }
}
