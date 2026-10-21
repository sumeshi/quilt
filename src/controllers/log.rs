use chrono::{DateTime, Local};
use log::{debug, info, warn};
pub struct LogController;
impl LogController {
    pub fn debug(msg: &str) {
        let timestamp = Self::get_timestamp();
        debug!("[{timestamp}] {msg}");
    }
    pub fn info(msg: &str) {
        let timestamp = Self::get_timestamp();
        info!("[{timestamp}] {msg}");
    }
    pub fn warn(msg: &str) {
        let timestamp = Self::get_timestamp();
        warn!("[{timestamp}] {msg}");
    }
    fn get_timestamp() -> String {
        let now: DateTime<Local> = Local::now();
        now.format("%Y-%m-%dT%H:%M:%S%.6f%:z").to_string()
    }
}
