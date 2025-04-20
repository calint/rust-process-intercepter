use std::env;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::unix::io::AsRawFd;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

fn main() -> io::Result<()> {
    // Get command arguments
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <command> [args...]", args[0]);
        return Ok(());
    }

    // Extract command and arguments
    let command = &args[1];
    let command_args = if args.len() > 2 {
        args[2..].to_vec()
    } else {
        vec![]
    };

    // Open log file
    let mut log_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("input_log.bin")?;

    // Spawn child process
    let mut child = Command::new(command)
        .args(&command_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;

    let stdin = child.stdin.take().expect("Failed to open stdin");

    // Create a channel for forwarding input
    let (tx, rx) = mpsc::channel();

    // Thread for reading raw binary input
    thread::spawn(move || {
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

        loop {
            // Try to read one character using getchar
            let c = unsafe { libc::getchar() };
            if c != -1 {
                // We got a character, send it immediately
                tx.send(c as u8).expect("Failed to send through channel");
            }
            // If c == -1, it means no data available, just continue the loop
        }
    });

    // Main thread handles receiving input and forwarding
    let mut process_stdin = stdin;

    for byte in rx {
        // Log to binary file
        log_file.write_all(&[byte])?;
        log_file.flush()?;

        // Forward to process
        process_stdin.write_all(&[byte])?;
        process_stdin.flush()?;
    }

    // Wait for the child process to complete
    let status = child.wait()?;
    println!("\nProcess exited with status: {}", status);

    Ok(())
}
