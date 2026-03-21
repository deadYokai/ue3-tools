use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::OnceLock,
};

static LOG: OnceLock<std::sync::Mutex<File>> = OnceLock::new();

pub fn init(exe_dir: &str) {
    let path = PathBuf::from(exe_dir).join("dinput8.log");
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => {
            let _ = LOG.set(std::sync::Mutex::new(f));
            raw_write(&format!(
                "[init] dinput8_wrapper loaded  (log: {})\n",
                path.display()
            ));
        }
        Err(e) => {
            let _ = e;
        }
    }
}

pub fn raw_write(line: &str) {
    if let Some(m) = LOG.get() {
        if let Ok(mut f) = m.lock() {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::log::raw_write(&format!("[info]  {}\n", format_args!($($arg)*)))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::log::raw_write(&format!("[warn]  {}\n", format_args!($($arg)*)))
    };
}

#[macro_export]
macro_rules! log_err {
    ($($arg:tt)*) => {
        $crate::log::raw_write(&format!("[error] {}\n", format_args!($($arg)*)))
    };
}
