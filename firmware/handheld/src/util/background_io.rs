use std::{
    fs::File,
    io::Read,
    sync::mpsc,
    time::{Duration, Instant},
};

pub enum ReaderResult {
    Ok(Vec<u8>),
    Eof,
    Err(std::io::Error),
}

pub struct BackgroundReader {
    receiver: mpsc::Receiver<ReaderResult>,
}

impl BackgroundReader {
    pub fn new(file: File, chunk_size: usize) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<ReaderResult>(0);

        let mut file = file;
        std::thread::spawn(move || {
            let mut duration = Duration::ZERO;

            loop {
                let mut buf = vec![0u8; chunk_size];
                let read_start = Instant::now();
                let result = file.read(&mut buf);
                duration += read_start.elapsed();

                let n = match result {
                    Ok(n) => n,
                    Err(err) => {
                        let _ = sender.send(ReaderResult::Err(err));
                        return;
                    }
                };
                if n == 0 {
                    log::info!("Read in {}ms", duration.as_millis());
                    let _ = sender.send(ReaderResult::Eof);
                    return;
                }
                buf.truncate(n);

                if let Err(_) = sender.send(ReaderResult::Ok(buf)) {
                    return;
                }
            }
        });

        BackgroundReader { receiver }
    }

    pub fn get(&mut self) -> ReaderResult {
        self.receiver.recv().unwrap_or(ReaderResult::Eof)
    }
}
