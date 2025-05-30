use std::fs::File;

use env_logger::{Builder, Env};

pub fn init_logging() {
    init_logging_file(None)
}

pub fn init_logging_file(output_file: Option<String>) {
    let target = match output_file {
        Some(f) => {
            let file = File::create(f).expect("couldn't create file");
            env_logger::Target::Pipe(Box::new(file))
        }
        None => env_logger::Target::Stderr,
    };

    Builder::from_env(Env::default().default_filter_or("info"))
        .format(|buf, record| {
            use std::io::Write;
            writeln!(
                buf,
                "{} [{}:{}] {}",
                record.level(),
                record.file().unwrap_or("unknown"),
                record.line().unwrap_or(0),
                record.args()
            )
        })
        .target(target)
        .init();
    log::info!("Logging initialized");
}
