pub mod minecraft;
use serenity::http::Http;
use std::{
    error::Error,
    sync::{
        mpsc::{self, SyncSender, TryRecvError},
        Arc, RwLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub trait Daemon {
    fn run(&self, http: &Http) -> Result<(), Box<dyn Error>>;
    fn interval(&self) -> Duration;
    fn name(&self) -> String;
}

#[derive(Debug)]
pub enum DaemonThreadMsg {
    RunAll,
    RunOne(usize),
}

pub struct DaemonThread {
    handle: JoinHandle<()>,
    channel: SyncSender<DaemonThreadMsg>,
    pub list: Vec<String>,
}

impl DaemonThread {
    pub fn run_one(&self, u: usize) -> Result<(), mpsc::SendError<DaemonThreadMsg>> {
        self.channel.send(DaemonThreadMsg::RunOne(u))?;
        self.handle.thread().unpark();
        Ok(())
    }

    pub fn run_all(&self) -> Result<(), mpsc::SendError<DaemonThreadMsg>> {
        self.channel.send(DaemonThreadMsg::RunAll)?;
        self.handle.thread().unpark();
        Ok(())
    }
}

impl serenity::prelude::TypeMapKey for DaemonThread {
    type Value = DaemonThread;
}

pub fn start_daemon_thread(
    daemons: Vec<Arc<RwLock<dyn Daemon + Send + Sync + 'static>>>,
    http: Arc<Http>,
) -> DaemonThread {
    let list = daemons
        .iter()
        .map(|d| d.read().unwrap().name())
        .collect::<Vec<_>>();
    let (sx, rx) = mpsc::sync_channel(512);
    let mut daemons = daemons
        .into_iter()
        .map(|d| (Instant::now(), d))
        .collect::<Vec<_>>();

    let mut next_global_run = None;
    let handle = thread::spawn(move || loop {
        match rx.try_recv() {
            Ok(DaemonThreadMsg::RunAll) => daemons.iter().for_each(|(_, d)| {
                let _ = d
                    .read()
                    .unwrap()
                    .run(&*http)
                    .map_err(|e| eprintln!("Deamon failed: {}", e));
            }),
            Ok(DaemonThreadMsg::RunOne(i)) => {
                if let Some(d) = daemons.get(i) {
                    let _ =
                        d.1.read()
                            .unwrap()
                            .run(&*http)
                            .map_err(|e| eprintln!("Deamon failed: {}", e));
                }
            }
            Err(TryRecvError::Empty) => {
                let mut smallest_next_instant = None;
                let now = Instant::now();
                for (next_run, daemon) in &mut daemons {
                    if now >= *next_run {
                        let d = daemon.read().unwrap();
                        let _ = d.run(&*http).map_err(|e| eprintln!("Deamon failed: {}", e));
                        *next_run = now + d.interval();
                    }
                    if smallest_next_instant.map(|s| *next_run < s).unwrap_or(true) {
                        smallest_next_instant = Some(*next_run)
                    }
                }
                match smallest_next_instant {
                    Some(s) => next_global_run = Some(s),
                    None => break eprintln!("Deamon thread terminating"),
                }
            }
            Err(_) => break eprintln!("Deamon thread terminating"),
        }
        let now = Instant::now();
        match next_global_run {
            Some(s) => thread::park_timeout(s - now),
            None => break eprintln!("Deamon thread terminating"),
        }
    });
    DaemonThread {
        handle,
        channel: sx,
        list,
    }
}
