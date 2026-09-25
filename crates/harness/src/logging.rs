use std::sync::Mutex;

static RECORDS: Mutex<Vec<String>> = Mutex::new(Vec::new());

// Logger that captures `log` records emitted while the runner executes.
struct CapturingLogger;

impl log::Log for CapturingLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Debug
    }

    fn log(&self, record: &log::Record) {
        let line = format!(
            "[{} {:<5} {}] {}",
            timestamp(),
            record.level(),
            record.target(),
            record.args()
        );
        RECORDS.lock().unwrap().push(line);
    }

    fn flush(&self) {}
}

pub(crate) fn init() {
    let _ = log::set_boxed_logger(Box::new(CapturingLogger));
    log::set_max_level(log::LevelFilter::Debug);
}

pub(crate) fn take_records() -> Vec<String> {
    std::mem::take(&mut RECORDS.lock().unwrap())
}

fn timestamp() -> String {
    let now = jiff::Timestamp::now();
    format!(
        "{}.{:09}Z",
        now.strftime("%Y-%m-%dT%H:%M:%S"),
        now.subsec_nanosecond()
    )
}
