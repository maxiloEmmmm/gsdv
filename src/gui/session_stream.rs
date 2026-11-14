//! Lightweight Agent session-file streaming for the remote Web UI.

use crate::gui::agent::AgentKind;
use crate::gui::terminal_host::TerminalSessionSource;
use axum::body::Bytes;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::mpsc;
use tokio::time::Instant;

/// Complete JSONL records sent when a browser opens a session for the first time.
const INITIAL_SESSION_LINES: usize = 100;
/// Maximum block read while locating the initial JSONL tail boundary.
const TAIL_SCAN_BLOCK_BYTES: u64 = 64 * 1024;
/// Minimum delay between recursive provider-directory searches.
const SESSION_LOCATE_RETRY: Duration = Duration::from_secs(1);

/// One Agent stream shared by all browser WebSockets for that Agent.
#[derive(Clone, Default)]
pub struct SessionStreamHub {
    /// Actor handles keyed by workspace and stable Agent id.
    streams: Arc<Mutex<HashMap<String, SessionStreamHandle>>>,
}

/// Metadata needed to attach one WebSocket to an Agent session stream.
pub struct SessionStreamDescriptor {
    /// Stable stream key scoped by workspace and Agent identity.
    pub stream_key: String,
    /// Agent activity source used only as a cheap file-stat trigger.
    pub source: TerminalSessionSource,
}

/// Messages emitted by a shared Agent stream actor.
pub enum SessionStreamMessage {
    /// The Agent switched to another persisted session.
    Reset {
        /// New session id, or an empty string while the hook has not reported it.
        session_id: String,
        /// Agent implementation used by the browser parser.
        agent_kind: AgentKind,
        /// Server-approved next byte, set to the initial tail after a fresh reset.
        seek: u64,
        /// File end observed when this reset was created.
        available_seek: u64,
    },
    /// Raw bytes from one persisted JSONL range.
    Chunk {
        /// Inclusive source-file byte offset.
        from_seek: u64,
        /// Exclusive source-file byte offset.
        to_seek: u64,
        /// Shared bytes reused by subscribers with the same seek.
        bytes: Bytes,
    },
}

/// One browser subscription to a shared Agent stream.
pub struct SessionStreamClient {
    /// Actor-local subscriber id.
    id: u64,
    /// Command mailbox shared with the Agent actor.
    command_tx: mpsc::UnboundedSender<SessionStreamCommand>,
    /// Server messages for this browser connection.
    pub messages: mpsc::Receiver<SessionStreamMessage>,
}

impl SessionStreamClient {
    /// Acknowledges a chunk after the WebSocket has written it.
    ///
    /// Example: binary frame ending at byte 4096 -> subscriber seek becomes 4096.
    pub fn acknowledge(&self, seek: u64) {
        let _ = self
            .command_tx
            .send(SessionStreamCommand::Acknowledge { id: self.id, seek });
    }
}

impl Drop for SessionStreamClient {
    /// Removes a closed browser from the shared Agent actor.
    ///
    /// Example: browser closes its tab -> subscriber no longer participates in reads.
    fn drop(&mut self) {
        let _ = self
            .command_tx
            .send(SessionStreamCommand::Unsubscribe { id: self.id });
    }
}

/// Cloneable mailbox handle for one Agent stream actor.
#[derive(Clone)]
struct SessionStreamHandle {
    /// Commands sent by every browser subscribed to this Agent.
    command_tx: mpsc::UnboundedSender<SessionStreamCommand>,
}

/// Commands consumed by the single owner of an Agent session fd.
enum SessionStreamCommand {
    /// Adds one browser cursor.
    Subscribe {
        /// Actor-local subscriber id.
        id: u64,
        /// Session id already held by the browser.
        session_id: String,
        /// Last byte already held by the browser.
        seek: u64,
        /// Bounded output queue for this browser.
        messages: mpsc::Sender<SessionStreamMessage>,
    },
    /// Advances a browser cursor after socket delivery.
    Acknowledge {
        /// Actor-local subscriber id.
        id: u64,
        /// Exclusive delivered file offset.
        seek: u64,
    },
    /// Removes one browser cursor.
    Unsubscribe {
        /// Actor-local subscriber id.
        id: u64,
    },
}

/// Mutable delivery state owned for one browser.
struct SessionSubscriber {
    /// Session id represented by `seek`.
    session_id: String,
    /// Next unread byte in the browser.
    seek: u64,
    /// Whether one chunk is waiting for socket acknowledgement.
    in_flight: bool,
    /// Whether current session metadata was delivered to this browser.
    reset_sent: bool,
    /// Bounded actor-to-socket queue.
    messages: mpsc::Sender<SessionStreamMessage>,
}

/// Provider/session identity reported by the Agent hook.
#[derive(Clone, PartialEq, Eq)]
struct SessionIdentity {
    /// Agent implementation used to locate and parse the file.
    agent_kind: AgentKind,
    /// Hook-reported persisted session id.
    session_id: String,
}

/// File identity and length captured by one metadata call.
#[derive(Clone, Copy, PartialEq, Eq)]
struct SessionFileState {
    /// Current file length in bytes.
    len: u64,
    /// Filesystem device id used to detect path replacement on Unix.
    #[cfg(unix)]
    dev: u64,
    /// Filesystem inode used to detect path replacement on Unix.
    #[cfg(unix)]
    ino: u64,
}

impl SessionFileState {
    /// Captures the fields needed to distinguish append, truncate, and replacement.
    ///
    /// Example: same inode with larger len -> ordinary append.
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;

        Self {
            len: metadata.len(),
            #[cfg(unix)]
            dev: metadata.dev(),
            #[cfg(unix)]
            ino: metadata.ino(),
        }
    }

    /// Returns whether the path still names the fd's original file.
    ///
    /// Example: atomic file replacement -> false on Unix.
    fn same_file(self, other: Self) -> bool {
        #[cfg(unix)]
        {
            self.dev == other.dev && self.ino == other.ino
        }
        #[cfg(not(unix))]
        {
            let _ = other;
            true
        }
    }
}

/// Open append-only session file owned exclusively by its Agent actor.
struct OpenSessionFile {
    /// Located provider file path used for the single metadata call.
    path: PathBuf,
    /// Reused asynchronous fd; normal appends never reopen it.
    file: File,
    /// Last path metadata observed after an Agent output signal.
    observed: SessionFileState,
}

/// Single owner of one Agent session fd and every browser seek.
struct SessionStreamActor {
    /// Agent output change source and hook session metadata.
    source: TerminalSessionSource,
    /// Browser subscribe/ack/close mailbox.
    commands: mpsc::UnboundedReceiver<SessionStreamCommand>,
    /// Browser delivery state keyed by actor-local id.
    subscribers: HashMap<u64, SessionSubscriber>,
    /// Hook identity currently represented by this actor.
    identity: Option<SessionIdentity>,
    /// Current provider session file, when it has appeared on disk.
    open_file: Option<OpenSessionFile>,
    /// Earliest time another recursive file lookup may run.
    next_locate_at: Instant,
}

impl SessionStreamHub {
    /// Subscribes one browser while reusing the existing per-Agent actor.
    ///
    /// Example: two fresh browsers -> one actor serving the last 100 JSONL records.
    pub fn subscribe(
        &self,
        descriptor: SessionStreamDescriptor,
        requested_session_id: String,
        requested_seek: u64,
    ) -> SessionStreamClient {
        let handle = {
            let mut streams = self
                .streams
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            streams
                .entry(descriptor.stream_key)
                .or_insert_with(|| spawn_session_stream_actor(descriptor.source))
                .clone()
        };
        let id = rand::random();
        let (messages, receiver) = mpsc::channel(4);
        let _ = handle.command_tx.send(SessionStreamCommand::Subscribe {
            id,
            session_id: requested_session_id,
            seek: requested_seek,
            messages,
        });
        SessionStreamClient {
            id,
            command_tx: handle.command_tx,
            messages: receiver,
        }
    }
}

/// Starts the single owner of one Agent session fd and subscriber seek table.
///
/// Example: first WebSocket for an Agent -> one Tokio actor.
fn spawn_session_stream_actor(source: TerminalSessionSource) -> SessionStreamHandle {
    let (command_tx, commands) = mpsc::unbounded_channel();
    let handle = SessionStreamHandle {
        command_tx: command_tx.clone(),
    };
    tokio::spawn(async move {
        SessionStreamActor {
            source,
            commands,
            subscribers: HashMap::new(),
            identity: None,
            open_file: None,
            next_locate_at: Instant::now(),
        }
        .run()
        .await;
    });
    handle
}

impl SessionStreamActor {
    /// Runs the Agent-change/stat/read loop and command mailbox.
    ///
    /// Example: PTY wakeup with unchanged file size -> one stat and no read.
    async fn run(&mut self) {
        loop {
            tokio::select! {
                changed = self.source.changed() => {
                    if !changed {
                        return;
                    }
                    if !self.subscribers.is_empty() {
                        self.refresh_from_agent_change().await;
                    }
                }
                command = self.commands.recv() => {
                    let Some(command) = command else {
                        return;
                    };
                    self.handle_command(command).await;
                }
            }
        }
    }

    /// Applies one browser subscribe/ack/close command.
    ///
    /// Example: chunk acknowledgement advances only that browser seek.
    async fn handle_command(&mut self, command: SessionStreamCommand) {
        match command {
            SessionStreamCommand::Subscribe {
                id,
                session_id,
                seek,
                messages,
            } => {
                self.subscribers.insert(
                    id,
                    SessionSubscriber {
                        session_id,
                        seek,
                        in_flight: false,
                        reset_sent: false,
                        messages,
                    },
                );
                self.refresh_from_agent_change().await;
                let invalid_seek = self
                    .open_file
                    .as_ref()
                    .and_then(|open| {
                        self.subscribers
                            .get(&id)
                            .map(|subscriber| subscriber.seek > open.observed.len)
                    })
                    .unwrap_or(false);
                self.send_reset_to(id, invalid_seek, None).await;
                self.distribute_available_bytes().await;
            }
            SessionStreamCommand::Acknowledge { id, seek } => {
                if let Some(subscriber) = self.subscribers.get_mut(&id) {
                    subscriber.seek = subscriber.seek.max(seek);
                    subscriber.in_flight = false;
                }
                self.distribute_available_bytes().await;
            }
            SessionStreamCommand::Unsubscribe { id } => {
                self.subscribers.remove(&id);
            }
        }
    }

    /// Stats the current session path once after an Agent output signal.
    ///
    /// Example: size 4096 -> 8192 triggers grouped reads from browser seeks.
    async fn refresh_from_agent_change(&mut self) {
        let metadata = self.source.metadata();
        let identity = SessionIdentity {
            agent_kind: metadata.agent_kind,
            session_id: metadata.session_id.unwrap_or_default(),
        };
        if self.identity.as_ref() != Some(&identity) {
            self.switch_session(identity).await;
            return;
        }
        if self.open_file.is_none() {
            self.open_current_session().await;
            if self.open_file.is_some() {
                // The hook may arrive before the JSONL path exists. Reset again
                // after opening so fresh browsers start at the final 100 records.
                self.broadcast_reset(true).await;
            }
            self.distribute_available_bytes().await;
            return;
        }

        let path = self
            .open_file
            .as_ref()
            .map(|open| open.path.clone())
            .expect("open file checked above");
        let Ok(metadata) = tokio::fs::metadata(&path).await else {
            self.open_file = None;
            return;
        };
        let observed = SessionFileState::from_metadata(&metadata);
        let previous = self
            .open_file
            .as_ref()
            .map(|open| open.observed)
            .expect("open file checked above");
        if !observed.same_file(previous) || observed.len < previous.len {
            self.open_file = None;
            self.open_current_session().await;
            self.broadcast_reset(true).await;
            self.distribute_available_bytes().await;
            return;
        }
        if observed.len == previous.len {
            return;
        }
        if let Some(open) = self.open_file.as_mut() {
            open.observed = observed;
        }
        self.distribute_available_bytes().await;
    }

    /// Switches hook identity, opens its file once, and resets browser cursors.
    ///
    /// Example: hook changes session a -> b -> all browsers restart b at zero.
    async fn switch_session(&mut self, identity: SessionIdentity) {
        self.identity = Some(identity);
        self.open_file = None;
        self.next_locate_at = Instant::now();
        self.open_current_session().await;
        self.broadcast_reset(true).await;
        self.distribute_available_bytes().await;
    }

    /// Locates and opens the current hook session without changing its identity.
    ///
    /// Example: hook arrives before JSONL creation -> later Agent signal opens it.
    async fn open_current_session(&mut self) {
        let Some(identity) = self.identity.as_ref() else {
            return;
        };
        if identity.session_id.is_empty() || Instant::now() < self.next_locate_at {
            return;
        }
        self.next_locate_at = Instant::now() + SESSION_LOCATE_RETRY;
        let Some(path) = locate_session_file(identity.agent_kind, &identity.session_id).await
        else {
            return;
        };
        let Ok(file) = File::open(&path).await else {
            return;
        };
        let Ok(metadata) = file.metadata().await else {
            return;
        };
        self.open_file = Some(OpenSessionFile {
            path,
            file,
            observed: SessionFileState::from_metadata(&metadata),
        });
    }

    /// Sends current identity to one subscriber unless it already received it.
    ///
    /// Example: fresh browser at seek 0 -> start from the final 100 JSONL records.
    async fn send_reset_to(
        &mut self,
        id: u64,
        force_seek_reset: bool,
        forced_initial_seek: Option<u64>,
    ) {
        let Some(identity) = self.identity.clone() else {
            return;
        };
        let available_seek = self
            .open_file
            .as_ref()
            .map(|open| open.observed.len)
            .unwrap_or(0);
        let Some(subscriber) = self.subscribers.get(&id) else {
            return;
        };
        if subscriber.reset_sent && !force_seek_reset {
            return;
        }
        let needs_initial_seek = force_seek_reset
            || subscriber.session_id != identity.session_id
            || subscriber.seek == 0;
        let preserved_seek = subscriber.seek;
        let initial_seek = if needs_initial_seek {
            match forced_initial_seek {
                Some(seek) => seek,
                None => self.initial_session_seek(available_seek).await,
            }
        } else {
            preserved_seek
        };
        let Some(subscriber) = self.subscribers.get_mut(&id) else {
            return;
        };
        if needs_initial_seek {
            subscriber.session_id.clone_from(&identity.session_id);
            subscriber.seek = initial_seek;
            subscriber.in_flight = false;
        }
        let sent = subscriber
            .messages
            .try_send(SessionStreamMessage::Reset {
                session_id: identity.session_id.clone(),
                agent_kind: identity.agent_kind,
                seek: subscriber.seek,
                available_seek,
            })
            .is_ok();
        subscriber.reset_sent = sent;
        if !sent {
            self.subscribers.remove(&id);
        }
    }

    /// Resets every browser and drops any slow client whose queue is full.
    ///
    /// Example: inode replacement -> every surviving browser seeks from zero.
    async fn broadcast_reset(&mut self, force_seek_reset: bool) {
        let forced_initial_seek = if force_seek_reset {
            let available_seek = self
                .open_file
                .as_ref()
                .map(|open| open.observed.len)
                .unwrap_or(0);
            Some(self.initial_session_seek(available_seek).await)
        } else {
            None
        };
        let ids = self.subscribers.keys().copied().collect::<Vec<_>>();
        for id in ids {
            if let Some(subscriber) = self.subscribers.get_mut(&id) {
                subscriber.reset_sent = false;
            }
            self.send_reset_to(id, force_seek_reset, forced_initial_seek)
                .await;
        }
    }

    /// Locates the first byte of the final complete JSONL records.
    ///
    /// Example: 10,000-line file -> seek at the beginning of line 9,901.
    async fn initial_session_seek(&mut self, file_size: u64) -> u64 {
        let Some(open) = self.open_file.as_mut() else {
            return 0;
        };
        tail_line_start(&mut open.file, file_size, INITIAL_SESSION_LINES)
            .await
            .unwrap_or(0)
    }

    /// Groups subscribers by seek and reads each complete live delta once.
    ///
    /// Example: seeks [4096, 4096, 8192] at size 12288 -> two reads to 12288.
    async fn distribute_available_bytes(&mut self) {
        let (session_id, observed_size) = match (&self.identity, &self.open_file) {
            (Some(identity), Some(open)) => (identity.session_id.clone(), open.observed.len),
            _ => return,
        };
        let mut groups = BTreeMap::<u64, Vec<u64>>::new();
        for (id, subscriber) in &self.subscribers {
            if subscriber.session_id == session_id
                && subscriber.reset_sent
                && !subscriber.in_flight
                && subscriber.seek < observed_size
            {
                groups.entry(subscriber.seek).or_default().push(*id);
            }
        }
        for (from_seek, ids) in groups {
            let to_seek = observed_size;
            let Some(open) = self.open_file.as_mut() else {
                return;
            };
            if open
                .file
                .seek(std::io::SeekFrom::Start(from_seek))
                .await
                .is_err()
            {
                continue;
            }
            let Ok(byte_count) = usize::try_from(to_seek - from_seek) else {
                continue;
            };
            let mut bytes = vec![0_u8; byte_count];
            if open.file.read_exact(&mut bytes).await.is_err() {
                continue;
            }
            let bytes = Bytes::from(bytes);
            for id in ids {
                let Some(subscriber) = self.subscribers.get_mut(&id) else {
                    continue;
                };
                if subscriber
                    .messages
                    .try_send(SessionStreamMessage::Chunk {
                        from_seek,
                        to_seek,
                        bytes: bytes.clone(),
                    })
                    .is_ok()
                {
                    subscriber.in_flight = true;
                }
            }
        }
    }
}

/// Scans backward in bounded blocks and returns the start of the final lines.
///
/// Example: `a\nb\nc\n` with two lines -> byte offset 2 (`b`).
async fn tail_line_start(
    file: &mut File,
    file_size: u64,
    line_count: usize,
) -> std::io::Result<u64> {
    if file_size == 0 || line_count == 0 {
        return Ok(file_size);
    }
    let mut cursor = file_size;
    let mut boundaries = 0_usize;
    while cursor > 0 {
        let block_start = cursor.saturating_sub(TAIL_SCAN_BLOCK_BYTES);
        let byte_count = usize::try_from(cursor - block_start).unwrap_or(0);
        let mut bytes = vec![0_u8; byte_count];
        file.seek(std::io::SeekFrom::Start(block_start)).await?;
        file.read_exact(&mut bytes).await?;
        for index in (0..bytes.len()).rev() {
            if bytes[index] != b'\n' {
                continue;
            }
            let absolute = block_start + index as u64;
            if absolute + 1 == file_size {
                continue;
            }
            boundaries += 1;
            if boundaries == line_count {
                return Ok(absolute + 1);
            }
        }
        cursor = block_start;
    }
    Ok(0)
}

/// Locates a Codex or Claude JSONL file when the Hook session id changes.
///
/// Example: Codex id under a dated rollout directory -> matching JSONL path.
async fn locate_session_file(agent_kind: AgentKind, session_id: &str) -> Option<PathBuf> {
    let session_id = session_id.to_string();
    tokio::task::spawn_blocking(move || {
        let home = crate::home::home_dir()?;
        let root = match agent_kind {
            AgentKind::Codex => std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"))
                .join("sessions"),
            AgentKind::Claude => home.join(".claude").join("projects"),
        };
        find_session_file(&root, &session_id, 0)
    })
    .await
    .ok()
    .flatten()
}

/// Recursively finds a matching session JSONL within bounded provider directories.
///
/// Example: depth 9 -> stop instead of scanning an unexpected tree forever.
fn find_session_file(root: &Path, session_id: &str, depth: usize) -> Option<PathBuf> {
    if depth > 8 {
        return None;
    }
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_session_file(&path, session_id, depth + 1) {
                return Some(found);
            }
            continue;
        }
        let matches = path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains(session_id));
        if matches {
            return Some(path);
        }
    }
    None
}
