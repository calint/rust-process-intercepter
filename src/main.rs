use std::env;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::unix::io::AsRawFd;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

const SLEEP_WHEN_NO_INPUT_MS: u64 = 10;

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
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    // take stdin to child
    let mut child_stdin = child.stdin.take().expect("stdin");
    let mut child_stdout = child.stdout.take().expect("stdout");

    // setup terminal for non-blocking, no echo
    setup_terminal();

    // thread for reading raw binary input and translating to serial and forwarding to child stdin and log file
    thread::spawn(move || {
        loop {
            // translate from terminal to serial
            match get_byte_non_blocking() {
                -1 => {
                    thread::sleep(Duration::from_millis(SLEEP_WHEN_NO_INPUT_MS));
                    continue;
                }
                0x0a => {
                    // carriage return
                    let slice = &[0x0d];
                    log_file.write_all(slice).expect("write to log");
                    child_stdin.write_all(slice).expect("write to child");
                }
                0x08 => {
                    // backspace
                    let slice = &[0x7f];
                    log_file.write_all(slice).expect("write to log");
                    child_stdin.write_all(slice).expect("write to child");
                }
                byte => {
                    let slice = &[byte as u8];
                    log_file.write_all(slice).expect("write to log");
                    child_stdin.write_all(slice).expect("write to child");
                }
            };
        }
    });

    // thread for reading child stdout and translating to console and forwarding to stdout
    thread::spawn(move || {
        loop {
            let mut buf = [0_u8; 1];
            // translate from terminal to serial
            child_stdout
                .read_exact(&mut buf)
                .expect("read from child stdout");
            match buf[0] {
                0x7f => {
                    // backspace
                    let mut stdout = io::stdout().lock();
                    stdout.write_all(b"\x08 \x08").expect("write to stdout");
                    stdout.flush().expect("flush stdout");
                }
                _ => {
                    let mut stdout = io::stdout().lock();
                    stdout.write_all(&buf).expect("write to stdout");
                    stdout.flush().expect("flush stdout");
                }
            };
        }
    });

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
