//! One persistent Play On worker per installed Jellyfin client.

use super::prepare::{self, PreparedPlay};
use super::protocol::{Action, Deduper, Playstate};
use super::websocket::{Frame, WebSocket, WsError};
use crate::jellyfin::client::JfClient;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const QUEUE_CAPACITY: usize = 16;
const DRAIN_BUDGET: usize = 8;

#[derive(Default)]
pub(super) struct Backoff {
    failures: u32,
}

impl Backoff {
    pub(super) fn next_delay(&mut self) -> Duration {
        let seconds = match self.failures {
            0 => 1,
            1 => 2,
            2 => 4,
            3 => 8,
            4 => 16,
            _ => 30,
        };
        self.failures = self.failures.saturating_add(1);
        Duration::from_secs(seconds)
    }

    pub(super) fn reset(&mut self) {
        self.failures = 0;
    }
}

pub(crate) enum RemoteCommand {
    Play(PreparedPlay),
    Playstate(Playstate),
}

struct Envelope {
    generation: u64,
    sequence: u64,
    command: RemoteCommand,
}

pub(crate) struct PlayOnAdapter {
    client: Option<*const JfClient>,
    generation: u64,
    receiver: Option<Receiver<Envelope>>,
    deferred: VecDeque<RemoteCommand>,
    stop: Option<Arc<AtomicBool>>,
    interrupt: Option<Arc<AtomicI32>>,
    worker: Option<JoinHandle<()>>,
    retired: Vec<JoinHandle<()>>,
}

impl Default for PlayOnAdapter {
    fn default() -> Self {
        Self {
            client: None,
            generation: 0,
            receiver: None,
            deferred: VecDeque::new(),
            stop: None,
            interrupt: None,
            worker: None,
            retired: Vec::new(),
        }
    }
}

impl PlayOnAdapter {
    pub(crate) fn sync(&mut self, client: Option<&'static JfClient>, enabled: bool) {
        self.reap();
        let requested = if enabled { client } else { None };
        let current = self.client;
        if current == requested.map(|client| client as *const JfClient) {
            return;
        }
        self.stop();
        let Some(client) = requested else {
            return;
        };
        self.generation = self.generation.wrapping_add(1).max(1);
        let generation = self.generation;
        let (sender, receiver) = std::sync::mpsc::sync_channel(QUEUE_CAPACITY);
        let stop = Arc::new(AtomicBool::new(false));
        let interrupt = Arc::new(AtomicI32::new(-1));
        let worker_stop = Arc::clone(&stop);
        let worker_interrupt = Arc::clone(&interrupt);
        self.client = Some(client as *const JfClient);
        self.receiver = Some(receiver);
        self.stop = Some(stop);
        self.interrupt = Some(interrupt);
        self.worker = crate::task::spawn_small_keeping("jellyfin-play-on", move || {
            run_worker(
                client,
                generation,
                worker_stop,
                worker_interrupt,
                sender,
            );
        });
    }

    pub(crate) fn drain(&mut self) -> Vec<RemoteCommand> {
        self.reap();
        let mut commands: Vec<RemoteCommand> = self.deferred.drain(..).collect();
        let Some(receiver) = self.receiver.as_ref() else {
            return commands;
        };
        let disconnected = drain_current(
            receiver,
            self.generation,
            DRAIN_BUDGET,
            &mut commands,
        );
        if disconnected {
            self.receiver.take();
            self.client = None;
        }
        commands
    }

    pub(crate) fn defer(&mut self, command: RemoteCommand) {
        if self.deferred.len() == QUEUE_CAPACITY {
            self.deferred.pop_front();
            crate::log("play on: deferred queue full; oldest command dropped");
        }
        self.deferred.push_back(command);
    }

    pub(crate) fn has_pending_play(&self) -> bool {
        self.deferred
            .iter()
            .any(|command| matches!(command, RemoteCommand::Play(_)))
    }

    pub(crate) fn stop(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.client = None;
        self.receiver.take();
        self.deferred.clear();
        if let Some(stop) = self.stop.take() {
            stop.store(true, Ordering::Release);
        }
        if let Some(interrupt) = self.interrupt.take() {
            let fd = interrupt.load(Ordering::Acquire);
            if fd >= 0 {
                unsafe { libc::shutdown(fd, libc::SHUT_RDWR) };
            }
        }
        if let Some(worker) = self.worker.take() {
            if worker.is_finished() {
                crate::task::join("jellyfin-play-on", worker);
            } else {
                self.retired.push(worker);
            }
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            crate::task::join("jellyfin-play-on", worker);
        }
        for worker in self.retired.drain(..) {
            crate::task::join("jellyfin-play-on", worker);
        }
    }

    fn reap(&mut self) {
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            if let Some(worker) = self.worker.take() {
                crate::task::join("jellyfin-play-on", worker);
            }
        }
        let mut index = 0;
        while index < self.retired.len() {
            if self.retired[index].is_finished() {
                let worker = self.retired.remove(index);
                crate::task::join("jellyfin-play-on", worker);
            } else {
                index += 1;
            }
        }
    }
}

impl Drop for PlayOnAdapter {
    fn drop(&mut self) {
        self.stop();
    }
}

fn drain_current(
    receiver: &Receiver<Envelope>,
    generation: u64,
    budget: usize,
    commands: &mut Vec<RemoteCommand>,
) -> bool {
    for _ in 0..budget {
        match receiver.try_recv() {
            Ok(envelope) if envelope.generation == generation => {
                let _sequence = envelope.sequence;
                commands.push(envelope.command);
            }
            Ok(_) => {}
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => return true,
        }
    }
    false
}

// Declared inside the Ok(socket) arm after the socket binding. Rust drops this guard first,
// so the published fd becomes -1 before RawConnection closes it, including break and panic paths.
struct InterruptReset<'a>(&'a AtomicI32);

impl Drop for InterruptReset<'_> {
    fn drop(&mut self) {
        self.0.store(-1, Ordering::Release);
    }
}

fn run_worker(
    client: &'static JfClient,
    generation: u64,
    stop: Arc<AtomicBool>,
    interrupt: Arc<AtomicI32>,
    sender: SyncSender<Envelope>,
) {    let mut backoff = Backoff::default();
    let mut deduper = Deduper::default();
    let mut sequence = 0_u64;
    while !stop.load(Ordering::Acquire) {
        match WebSocket::connect(client) {
            Ok(mut socket) => {
                interrupt.store(socket.fd(), Ordering::Release);
                let _reset_interrupt = InterruptReset(&interrupt);
                if stop.load(Ordering::Acquire) {
                    break;
                }
                if !client.post_play_on_capabilities() {
                    crate::log("play on: capabilities refused; reconnecting");
                } else {
                    backoff.reset();
                    run_connection(
                        client,
                        generation,
                        &stop,
                        &mut socket,
                        &sender,
                        &mut deduper,
                        &mut sequence,
                    );
                }
            }
            Err(_) => crate::log("play on: WebSocket connect failed"),
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        interruptible_sleep(&stop, backoff.next_delay());
    }
    interrupt.store(-1, Ordering::Release);
}

#[allow(clippy::too_many_arguments)]
fn run_connection(
    client: &JfClient,
    generation: u64,
    stop: &AtomicBool,
    socket: &mut WebSocket,
    sender: &SyncSender<Envelope>,
    deduper: &mut Deduper,
    sequence: &mut u64,
) {
    let mut interval = Duration::from_secs(30);
    let mut next_keepalive = Instant::now() + interval;
    while !stop.load(Ordering::Acquire) {
        let timeout = Duration::from_millis(100).min(
            next_keepalive
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO),
        );
        match socket.next_frame(timeout) {
            Ok(Frame::Text(json)) => {
                let Ok(message) = super::protocol::parse_detailed(&json) else {
                    crate::log("play on: invalid command ignored");
                    continue;
                };
                if !deduper.accept(&message) {
                    continue;
                }
                match message.action {
                    Some(Action::ForceKeepAlive(seconds)) => {
                        interval = Duration::from_secs(seconds);
                        if socket.send_text(KEEPALIVE).is_err() {
                            break;
                        }
                        next_keepalive = Instant::now() + interval;
                    }
                    Some(Action::Playstate(state)) => {
                        if !publish(sender, generation, sequence, RemoteCommand::Playstate(state)) {
                            return;
                        }
                    }
                    Some(Action::Play(request)) => {
                        if let Some(prepared) = prepare::fetch(client, &request) {
                            if !publish(sender, generation, sequence, RemoteCommand::Play(prepared)) {
                                return;
                            }
                        } else {
                            crate::log("play on: PlayNow item or source refused");
                        }
                    }
                    None => {}
                }
            }
            Ok(Frame::Ping(payload)) => {
                if socket.send_frame(0x0a, &payload).is_err() {
                    break;
                }
            }
            Err(WsError::Io(error)) if error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => break,
        }
        if Instant::now() >= next_keepalive {
            if socket.send_text(KEEPALIVE).is_err() {
                break;
            }
            next_keepalive = Instant::now() + interval;
        }
    }
}

const KEEPALIVE: &str = r#"{"MessageType":"KeepAlive"}"#;

fn publish(
    sender: &SyncSender<Envelope>,
    generation: u64,
    sequence: &mut u64,
    command: RemoteCommand,
) -> bool {
    *sequence = sequence.wrapping_add(1);
    match sender.try_send(Envelope {
        generation,
        sequence: *sequence,
        command,
    }) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            crate::log("play on: command queue full; command dropped");
            true
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}

fn interruptible_sleep(stop: &AtomicBool, duration: Duration) {
    let deadline = Instant::now() + duration;
    while !stop.load(Ordering::Acquire) {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        if remaining.is_zero() {
            break;
        }
        std::thread::sleep(remaining.min(Duration::from_millis(100)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_on_drain_discards_stale_generations() {
        let (sender, receiver) = std::sync::mpsc::sync_channel(2);
        sender
            .send(Envelope {
                generation: 1,
                sequence: 1,
                command: RemoteCommand::Playstate(Playstate::Stop),
            })
            .unwrap();
        sender
            .send(Envelope {
                generation: 2,
                sequence: 1,
                command: RemoteCommand::Playstate(Playstate::Pause),
            })
            .unwrap();
        let mut commands = Vec::new();
        assert!(!drain_current(&receiver, 2, 8, &mut commands));
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands.first(),
            Some(RemoteCommand::Playstate(Playstate::Pause))
        ));
    }

    #[test]
    fn play_on_deferred_playstates_keep_their_order() {
        let mut adapter = PlayOnAdapter::default();
        adapter.defer(RemoteCommand::Playstate(Playstate::Pause));
        adapter.defer(RemoteCommand::Playstate(Playstate::Stop));
        let commands = adapter.drain();
        assert_eq!(commands.len(), 2);
        assert!(matches!(
            commands.first(),
            Some(RemoteCommand::Playstate(Playstate::Pause))
        ));
        assert!(matches!(
            commands.get(1),
            Some(RemoteCommand::Playstate(Playstate::Stop))
        ));
    }
}
